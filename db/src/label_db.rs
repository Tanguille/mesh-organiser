use indexmap::IndexMap;
use itertools::{Itertools, join};
use sqlx::Row;

use crate::{
    DbError,
    db_context::DbContext,
    model::{
        label::{Label, LabelMeta},
        user::User,
    },
    model_db, random_hex_32,
    util::time_now,
};

/// Builds a batch INSERT query string for N-column values.
/// Used to efficiently insert multiple rows in a single query.
fn build_batch_insert_query(
    table: &str,
    columns: &[&str],
    values: &[impl std::fmt::Display],
) -> String {
    let columns_joined = columns.join(", ");
    let values_clause = values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("INSERT INTO {table} ({columns_joined}) VALUES {values_clause}")
}

/// Formats a tuple of values for SQL VALUES clause.
fn sql_tuple(values: &[impl std::fmt::Display]) -> String {
    let inner = values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!("({inner})")
}

pub async fn get_labels_min(db: &DbContext) -> Result<Vec<LabelMeta>, DbError> {
    let rows = sqlx::query!("SELECT label_id, label_name, label_color, label_unique_global_id, label_last_modified FROM labels")
        .fetch_all(db)
        .await?;

    let mut labels = Vec::new();

    for row in rows {
        labels.push(LabelMeta {
            id: row.label_id,
            name: row.label_name,
            color: row.label_color,
            unique_global_id: row.label_unique_global_id,
            last_modified: row.label_last_modified,
        });
    }

    Ok(labels)
}

fn get_effective_labels(
    label_id: i64,
    effective_labels: &mut Vec<LabelMeta>,
    label_map: &mut IndexMap<i64, Label>,
) {
    let label = label_map.get(&label_id).unwrap();

    if !effective_labels.iter().any(|l| l.id == label.meta.id) {
        effective_labels.push(label.meta.clone());
    }

    let label_child_ids = label.children.iter().map(|l| l.id).collect::<Vec<i64>>();

    for id in &label_child_ids {
        if !effective_labels.iter().any(|l| &l.id == id) {
            get_effective_labels(*id, effective_labels, label_map);
        }
    }
}

/// Fills `effective_labels`, `group_count`, and `model_count` for each label using a key snapshot
/// so that `label_map` can be mutated in the loop (borrow checker).
fn fill_effective_labels_and_counts(label_map: &mut IndexMap<i64, Label>) {
    let label_ids: Vec<i64> = label_map.keys().copied().collect();
    for label_id in label_ids {
        let mut effective_labels = Vec::new();
        get_effective_labels(label_id, &mut effective_labels, label_map);
        let group_count = effective_labels
            .iter()
            .map(|l| label_map.get(&l.id).unwrap().self_group_count)
            .sum();
        let model_count = effective_labels
            .iter()
            .map(|l| label_map.get(&l.id).unwrap().self_model_count)
            .sum();
        let label = label_map.get_mut(&label_id).unwrap();
        label.effective_labels = effective_labels;
        label.group_count = group_count;
        label.model_count = model_count;
    }
}

pub async fn get_labels(
    db: &DbContext,
    user: &User,
    include_ungrouped_models: bool,
) -> Result<Vec<Label>, DbError> {
    // Use a CTE with JOIN-based aggregations instead of correlated subqueries
    // This eliminates N+1 query patterns and improves performance
    let rows = sqlx::query(
    "WITH label_stats AS (
        SELECT 
            labels.label_id,
            COUNT(DISTINCT models_labels.model_id) as model_count,
            COUNT(DISTINCT models.model_group_id) as group_count,
            COUNT(DISTINCT CASE WHEN models.model_group_id IS NULL THEN models_labels.model_id END) as ungrouped_count
        FROM labels
        LEFT JOIN models_labels ON labels.label_id = models_labels.label_id
        LEFT JOIN models ON models_labels.model_id = models.model_id
        WHERE labels.label_user_id = ?
        GROUP BY labels.label_id
    )
    SELECT
            parent_labels.label_id  as parent_label_id,
            parent_labels.label_name as parent_label_name,
            parent_labels.label_color as parent_label_color,
            parent_labels.label_unique_global_id as parent_label_unique_global_id,
            parent_labels.label_last_modified as parent_label_last_modified,
            COALESCE(label_stats.model_count, 0) as parent_label_model_count,
            COALESCE(label_stats.group_count, 0) as parent_label_group_count,
            COALESCE(label_stats.ungrouped_count, 0) as parent_label_ungrouped_count,
            child_labels.label_id as child_label_id,
            child_labels.label_name as child_label_name,
            child_labels.label_color as child_label_color,
            child_labels.label_unique_global_id as child_label_unique_global_id
          FROM labels as parent_labels
          LEFT JOIN label_stats ON parent_labels.label_id = label_stats.label_id
          LEFT JOIN labels_labels ON parent_labels.label_id = labels_labels.parent_label_id
          LEFT JOIN labels as child_labels ON labels_labels.child_label_id = child_labels.label_id
          WHERE parent_labels.label_user_id = ?
          ORDER BY parent_labels.label_name ASC"
    )
    .bind(user.id)
    .fetch_all(db)
    .await?;

    let mut label_map: IndexMap<i64, Label> = IndexMap::new();
    let mut has_parents = vec![];

    for row in rows {
        let parent_label_id: i64 = row.get("parent_label_id");
        let parent_label_name: String = row.get("parent_label_name");
        let parent_label_color: i64 = row.get("parent_label_color");
        let parent_label_unique_global_id: String = row.get("parent_label_unique_global_id");
        let parent_label_last_modified: String = row.get("parent_label_last_modified");

        let parent_label_model_count: i64 = row.get("parent_label_model_count");
        let parent_label_group_count: i64 = row.get("parent_label_group_count");
        let parent_label_ungrouped_count: i64 = row.get("parent_label_ungrouped_count");

        let child_label_id: Option<i64> = row.get("child_label_id");
        let child_label_name: Option<String> = row.get("child_label_name");
        let child_label_color: Option<i64> = row.get("child_label_color");
        let child_label_unique_global_id: Option<String> = row.get("child_label_unique_global_id");

        let entry = label_map.entry(parent_label_id).or_insert(Label {
            meta: LabelMeta {
                id: parent_label_id,
                name: parent_label_name,
                color: parent_label_color,
                unique_global_id: parent_label_unique_global_id,
                last_modified: parent_label_last_modified,
            },
            children: Vec::new(),
            effective_labels: Vec::new(),
            has_parent: false,
            model_count: parent_label_model_count,
            group_count: parent_label_group_count,
            self_model_count: parent_label_model_count,
            self_group_count: parent_label_group_count,
        });

        if include_ungrouped_models {
            entry.self_group_count += parent_label_ungrouped_count;
            entry.group_count += parent_label_ungrouped_count;
        }

        if let Some(child_id) = child_label_id
            && child_id > 0
        {
            entry.children.push(LabelMeta {
                id: child_id,
                name: child_label_name.unwrap(),
                color: child_label_color.unwrap(),
                unique_global_id: child_label_unique_global_id.unwrap(),
                last_modified: String::new(),
            });

            has_parents.push(child_id);
        }
    }

    for entry in has_parents {
        if let Some(label) = label_map.get_mut(&entry) {
            label.has_parent = true;
        }
    }

    fill_effective_labels_and_counts(&mut label_map);

    Ok(label_map.into_values().collect())
}

pub async fn get_unique_id_from_label_id(
    db: &DbContext,
    user: &User,
    label_id: i64,
) -> Result<String, DbError> {
    let row = sqlx::query!(
        "SELECT label_unique_global_id FROM labels WHERE label_id = ? AND label_user_id = ?",
        label_id,
        user.id
    )
    .fetch_one(db)
    .await?;

    Ok(row.label_unique_global_id)
}

pub async fn get_unique_ids_from_label_ids(
    db: &DbContext,
    user: &User,
    label_ids: &[i64],
) -> Result<IndexMap<i64, String>, DbError> {
    let ids_placeholder = join(label_ids.iter(), ",");

    let query = format!(
        "SELECT label_id, label_unique_global_id FROM labels WHERE label_id IN ({ids_placeholder}) AND label_user_id = ?"
    );

    let rows = sqlx::query(&query).bind(user.id).fetch_all(db).await?;

    let mut id_map = IndexMap::new();
    for row in rows {
        let label_id: i64 = row.get("label_id");
        let label_unique_global_id: String = row.get("label_unique_global_id");
        id_map.insert(label_id, label_unique_global_id);
    }

    Ok(id_map)
}

pub async fn add_labels_on_models(
    db: &DbContext,
    user: &User,
    label_ids: &[i64],
    model_ids: &[i64],
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    // Batch permission check for all labels
    let label_global_ids = get_unique_ids_from_label_ids(db, user, label_ids).await?;
    if label_global_ids.values().len() != label_ids.len() {
        return Err(DbError::RowNotFound);
    }

    // Collect all label-model combinations
    let mut values = Vec::with_capacity(label_ids.len() * model_ids.len());
    for label_id in label_ids {
        for model_id in model_ids {
            values.push((*label_id, *model_id));
        }
    }

    // Batch insert using a single query with multiple VALUES
    if !values.is_empty() {
        let tuples: Vec<String> = values.iter().map(|(l, m)| sql_tuple(&[*l, *m])).collect();
        let query = build_batch_insert_query("models_labels", &["label_id", "model_id"], &tuples);
        sqlx::query(&query).execute(db).await?;
    }

    // Batch update timestamps
    set_last_updated_on_labels(db, user, label_ids, update_timestamp.unwrap_or(&time_now()))
        .await?;

    Ok(())
}

pub async fn remove_labels_from_models(
    db: &DbContext,
    user: &User,
    label_ids: &[i64],
    model_ids: &[i64],
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let label_global_ids = get_unique_ids_from_label_ids(db, user, label_ids).await?;

    if label_global_ids.len() != label_ids.len() {
        return Err(DbError::RowNotFound);
    }

    let model_global_ids = model_db::get_unique_ids_from_model_ids(db, model_ids.to_vec()).await?;

    if model_global_ids.len() != model_ids.len() {
        return Err(DbError::RowNotFound);
    }

    let joined_labels = join(label_ids.iter(), ",");
    let joined_models = join(model_ids.iter(), ",");

    let formatted_query = format!(
        "DELETE FROM models_labels WHERE label_id IN ({joined_labels}) AND model_id IN ({joined_models})"
    );

    sqlx::query(&formatted_query).execute(db).await?;

    set_last_updated_on_labels(db, user, label_ids, update_timestamp.unwrap_or(&time_now()))
        .await?;

    Ok(())
}

pub async fn remove_all_labels_from_models(
    db: &DbContext,
    user: &User,
    model_ids: &[i64],
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let models = model_db::get_models_via_ids(db, user, model_ids.to_vec()).await?;

    if models.len() != model_ids.len() {
        return Err(DbError::RowNotFound);
    }

    let joined_models = join(models.iter().map(|f| f.id), ",");

    let formatted_query = format!("DELETE FROM models_labels WHERE model_id IN ({joined_models})");

    sqlx::query(&formatted_query).execute(db).await?;

    let label_ids: Vec<i64> = models
        .iter()
        .flat_map(|m| m.labels.iter().map(|l| l.id))
        .unique()
        .collect();
    set_last_updated_on_labels(
        db,
        user,
        &label_ids,
        update_timestamp.unwrap_or(&time_now()),
    )
    .await?;

    Ok(())
}

pub async fn add_label(
    db: &DbContext,
    user: &User,
    name: &str,
    color: i64,
    update_timestamp: Option<&str>,
) -> Result<i64, DbError> {
    let unique_global_id = random_hex_32();
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);

    let result = sqlx::query!(
        "INSERT INTO labels (label_name, label_color, label_user_id, label_unique_global_id, label_last_modified) VALUES (?, ?, ?, ?, ?)",
        name,
        color,
        user.id,
        unique_global_id,
        timestamp
    )
    .execute(db)
    .await?;

    let label_id = result.last_insert_rowid();
    Ok(label_id)
}

pub async fn edit_label(
    db: &DbContext,
    user: &User,
    label_id: i64,
    name: &str,
    color: i64,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);

    sqlx::query!(
        "UPDATE labels SET label_name = ?, label_color = ?, label_last_modified = ? WHERE label_id = ? AND label_user_id = ?",
        name,
        color,
        timestamp,
        label_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn edit_label_global_id(
    db: &DbContext,
    user: &User,
    label_id: i64,
    unique_global_id: &str,
) -> Result<(), DbError> {
    if unique_global_id.len() != 32 {
        return Err(DbError::InvalidArgument(
            "Unique Global ID must be 32 characters long".to_string(),
        ));
    }

    sqlx::query!(
        "UPDATE labels SET label_unique_global_id = ? WHERE label_id = ? AND label_user_id = ?",
        unique_global_id,
        label_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn delete_label(db: &DbContext, user: &User, label_id: i64) -> Result<(), DbError> {
    sqlx::query!(
        "DELETE FROM labels WHERE label_id = ? AND label_user_id = ?",
        label_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn add_childs_to_label(
    db: &DbContext,
    user: &User,
    parent_label_id: i64,
    child_label_ids: Vec<i64>,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);
    let _parent_hex = get_unique_id_from_label_id(db, user, parent_label_id).await?;
    let access_check = get_unique_ids_from_label_ids(db, user, &child_label_ids).await?;

    if access_check.len() != child_label_ids.len() {
        return Err(DbError::RowNotFound);
    }

    // Batch insert using a single query with multiple VALUES
    if !child_label_ids.is_empty() {
        let tuples: Vec<String> = child_label_ids
            .iter()
            .map(|child_id| sql_tuple(&[parent_label_id, *child_id]))
            .collect();
        let query = build_batch_insert_query(
            "labels_labels",
            &["parent_label_id", "child_label_id"],
            &tuples,
        );
        sqlx::query(&query).execute(db).await?;
    }

    set_last_updated_on_label(db, user, parent_label_id, timestamp).await?;

    Ok(())
}

pub async fn remove_childs_from_label(
    db: &DbContext,
    user: &User,
    parent_label_id: i64,
    child_label_ids: Vec<i64>,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);
    let _parent_hex = get_unique_id_from_label_id(db, user, parent_label_id).await?;
    let access_check = get_unique_ids_from_label_ids(db, user, &child_label_ids).await?;

    if access_check.len() != child_label_ids.len() {
        return Err(DbError::RowNotFound);
    }

    // Batch delete using IN clause
    if !child_label_ids.is_empty() {
        let child_ids_placeholder = join(child_label_ids.iter(), ",");
        let query = format!(
            "DELETE FROM labels_labels WHERE parent_label_id = ? AND child_label_id IN ({child_ids_placeholder})"
        );

        sqlx::query(&query)
            .bind(parent_label_id)
            .execute(db)
            .await?;
    }

    set_last_updated_on_label(db, user, parent_label_id, timestamp).await?;
    Ok(())
}

pub async fn remove_all_childs_from_label(
    db: &DbContext,
    user: &User,
    parent_label_id: i64,
    update_timestamp: Option<&str>,
) -> Result<(), DbError> {
    let now = time_now();
    let timestamp = update_timestamp.unwrap_or(&now);
    let _unique_global_id = get_unique_id_from_label_id(db, user, parent_label_id).await?;

    sqlx::query!(
        "DELETE FROM labels_labels WHERE parent_label_id = ?",
        parent_label_id
    )
    .execute(db)
    .await?;

    set_last_updated_on_label(db, user, parent_label_id, timestamp).await?;

    Ok(())
}

pub async fn set_last_updated_on_label(
    db: &DbContext,
    user: &User,
    label_id: i64,
    timestamp: &str,
) -> Result<(), DbError> {
    sqlx::query!(
        "UPDATE labels SET label_last_modified = ? WHERE label_id = ? AND label_user_id = ?",
        timestamp,
        label_id,
        user.id
    )
    .execute(db)
    .await?;

    Ok(())
}

pub async fn set_last_updated_on_labels(
    db: &DbContext,
    user: &User,
    label_ids: &[i64],
    timestamp: &str,
) -> Result<(), DbError> {
    let ids_placeholder = join(label_ids.iter(), ",");

    let query = format!(
        "UPDATE labels SET label_last_modified = ? WHERE label_id IN ({ids_placeholder}) AND label_user_id = ?"
    );

    sqlx::query(&query)
        .bind(timestamp)
        .bind(user.id)
        .execute(db)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    //! Tests for `get_labels` / effective-labels logic: key-snapshot loop and `fill_effective_labels_and_counts`.
    //! Verifies (a) label with no children has `effective_labels` = [self] and counts = self_*,
    //! (b) label with children has `effective_labels` = self + descendants, (c) `group_count`/`model_count`
    //! are sums of self_* over effective labels.

    use indexmap::IndexMap;

    use crate::model::label::{Label, LabelMeta};

    use super::fill_effective_labels_and_counts;

    fn meta(id: i64, name: &str) -> LabelMeta {
        LabelMeta {
            id,
            name: name.to_string(),
            color: 0,
            unique_global_id: format!("{id:032x}"),
            last_modified: String::new(),
        }
    }

    fn label_with_children(
        meta: LabelMeta,
        children: Vec<LabelMeta>,
        self_group_count: i64,
        self_model_count: i64,
    ) -> Label {
        Label {
            meta,
            children,
            effective_labels: Vec::new(),
            has_parent: false,
            model_count: self_model_count,
            group_count: self_group_count,
            self_model_count,
            self_group_count,
        }
    }

    #[test]
    fn effective_labels_leaf_label_contains_only_self_and_counts_equal_self() {
        let mut label_map: IndexMap<i64, Label> = IndexMap::new();
        let leaf_meta = meta(1, "Leaf");
        label_map.insert(1, label_with_children(leaf_meta, vec![], 10, 20));

        fill_effective_labels_and_counts(&mut label_map);

        let label = label_map.get(&1).unwrap();
        assert_eq!(label.effective_labels.len(), 1);
        assert_eq!(label.effective_labels[0].id, 1);
        assert_eq!(label.group_count, 10);
        assert_eq!(label.model_count, 20);
    }

    #[test]
    fn effective_labels_parent_includes_self_and_descendants() {
        let mut label_map: IndexMap<i64, Label> = IndexMap::new();
        let parent_meta = meta(1, "Parent");
        let child_meta = meta(2, "Child");
        label_map.insert(
            1,
            label_with_children(parent_meta, vec![child_meta.clone()], 1, 2),
        );
        label_map.insert(2, label_with_children(child_meta, vec![], 3, 5));

        fill_effective_labels_and_counts(&mut label_map);

        let parent = label_map.get(&1).unwrap();
        assert_eq!(parent.effective_labels.len(), 2);
        assert_eq!(parent.effective_labels[0].id, 1);
        assert_eq!(parent.effective_labels[1].id, 2);
        assert_eq!(parent.group_count, 1 + 3);
        assert_eq!(parent.model_count, 2 + 5);

        let child = label_map.get(&2).unwrap();
        assert_eq!(child.effective_labels.len(), 1);
        assert_eq!(child.effective_labels[0].id, 2);
        assert_eq!(child.group_count, 3);
        assert_eq!(child.model_count, 5);
    }

    #[test]
    fn effective_labels_three_level_hierarchy_counts_sum_over_effective() {
        let mut label_map: IndexMap<i64, Label> = IndexMap::new();
        let root_meta = meta(1, "Root");
        let mid_meta = meta(2, "Mid");
        let leaf_meta = meta(3, "Leaf");
        label_map.insert(
            1,
            label_with_children(root_meta, vec![mid_meta.clone()], 1, 10),
        );
        label_map.insert(
            2,
            label_with_children(mid_meta, vec![leaf_meta.clone()], 2, 20),
        );
        label_map.insert(3, label_with_children(leaf_meta, vec![], 4, 40));

        fill_effective_labels_and_counts(&mut label_map);

        let root = label_map.get(&1).unwrap();
        assert_eq!(root.effective_labels.len(), 3);
        assert_eq!(
            root.effective_labels
                .iter()
                .map(|l| l.id)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(root.group_count, 1 + 2 + 4);
        assert_eq!(root.model_count, 10 + 20 + 40);

        let mid = label_map.get(&2).unwrap();
        assert_eq!(mid.effective_labels.len(), 2);
        assert_eq!(mid.group_count, 2 + 4);
        assert_eq!(mid.model_count, 20 + 40);

        let leaf = label_map.get(&3).unwrap();
        assert_eq!(leaf.effective_labels.len(), 1);
        assert_eq!(leaf.group_count, 4);
        assert_eq!(leaf.model_count, 40);
    }
}
