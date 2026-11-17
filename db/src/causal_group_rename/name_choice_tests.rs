//! A received name resolution must choose from the names visible in its ancestry.

use crate::{causal_genesis, group_db, sync_peer, user_db};

use super::*;

async fn test_db() -> (tempfile::TempDir, DbContext) {
    let dir = tempfile::tempdir().unwrap();
    let backup = dir.path().join("backups");
    std::fs::create_dir_all(&backup).unwrap();
    let db = crate::db_context::setup_db(&dir.path().join("db.sqlite"), &backup).await;
    (dir, db)
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // Keep genuine and forged resolutions in one shared history.
async fn forged_import_cannot_choose_unoffered_name() {
    let (_ad, a) = test_db().await;
    let (_bd, b) = test_db().await;
    let (_cd, c) = test_db().await;
    let user = User::default();
    let ai = user_db::get_sync_identity(&a, user.id)
        .await
        .unwrap()
        .unwrap();
    let bi = user_db::get_sync_identity(&b, user.id)
        .await
        .unwrap()
        .unwrap();
    let ci = user_db::get_sync_identity(&c, user.id)
        .await
        .unwrap()
        .unwrap();
    let group = group_db::add_empty_group(&a, &user, "original", None)
        .await
        .unwrap();
    let root = causal_genesis::enroll_pristine_group(&a, &user, &ai, group.id)
        .await
        .unwrap();
    let ba = sync_peer::bind_peer(&b, &user, &bi, &ai).await.unwrap();
    let ca = sync_peer::bind_peer(&c, &user, &ci, &ai).await.unwrap();
    let bg = causal_genesis::install_pristine_genesis(&b, &user, &bi, &ba, &root, None)
        .await
        .unwrap()
        .local_id;
    let cg = causal_genesis::install_pristine_genesis(&c, &user, &ci, &ca, &root, None)
        .await
        .unwrap()
        .local_id;
    group_db::edit_group(&a, &user, group.id, "intermediate", None, None)
        .await
        .unwrap();
    let NameState::Ready { token: shared, .. } = get_state(&a, &user, &ai, group.id).await.unwrap()
    else {
        panic!("expected shared ancestor")
    };
    let ancestor = export_event(&a, &user, &ai, &shared.frontier[0])
        .await
        .unwrap();
    import_event(&b, &user, &bi, &ba, &ancestor).await.unwrap();
    import_event(&c, &user, &ci, &ca, &ancestor).await.unwrap();
    group_db::edit_group(&a, &user, group.id, "alpha", None, None)
        .await
        .unwrap();
    group_db::edit_group(&b, &user, bg, "bravo", None, None)
        .await
        .unwrap();
    let NameState::Ready { token: at, .. } = get_state(&a, &user, &ai, group.id).await.unwrap()
    else {
        panic!("expected alpha")
    };
    let NameState::Ready { token: bt, .. } = get_state(&b, &user, &bi, bg).await.unwrap() else {
        panic!("expected bravo")
    };
    let ae = export_event(&a, &user, &ai, &at.frontier[0]).await.unwrap();
    let be = export_event(&b, &user, &bi, &bt.frontier[0]).await.unwrap();
    let ab = sync_peer::bind_peer(&a, &user, &ai, &bi).await.unwrap();
    let cb = sync_peer::bind_peer(&c, &user, &ci, &bi).await.unwrap();
    import_event(&a, &user, &ai, &ab, &be).await.unwrap();
    import_event(&c, &user, &ci, &ca, &ae).await.unwrap();
    import_event(&c, &user, &ci, &cb, &be).await.unwrap();
    let NameState::Conflict { token, .. } = get_state(&a, &user, &ai, group.id).await.unwrap()
    else {
        panic!("expected conflict")
    };
    let ResolveResult::Resolved(NameState::Ready { token, .. }) =
        resolve(&a, &user, &ai, group.id, &token, "alpha")
            .await
            .unwrap()
    else {
        panic!("expected offered resolution")
    };
    let valid = export_event(&a, &user, &ai, &token.frontier[0])
        .await
        .unwrap();
    let mut forged = valid.envelope.clone();
    let Action::GroupNameResolved { name } = &mut forged.action else {
        panic!("expected resolution")
    };
    *name = "unoffered".into();
    let wire = causal_envelope::canonical_causal_envelope(&forged).unwrap();
    let forged = RenameBundle {
        envelope: forged,
        source_identity: valid.source_identity.clone(),
        root: valid.root.clone(),
        wire,
    };
    let before = get_state(&c, &user, &ci, cg).await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&c)
        .await
        .unwrap();
    assert!(
        import_event(&c, &user, &ci, &ca, &forged).await.is_err(),
        "unoffered resolution must be rejected"
    );
    assert_eq!(get_state(&c, &user, &ci, cg).await.unwrap(), before);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&c)
        .await
        .unwrap();
    assert_eq!(after, count);
    let mut transitive = valid.envelope.clone();
    transitive.parents.push(shared.frontier[0].clone());
    let Action::GroupNameResolved { name } = &mut transitive.action else {
        panic!("expected resolution")
    };
    *name = "intermediate".into();
    let wire = causal_envelope::canonical_causal_envelope(&transitive).unwrap();
    let transitive = RenameBundle {
        envelope: transitive,
        source_identity: valid.source_identity.clone(),
        root: valid.root.clone(),
        wire,
    };
    assert!(
        import_event(&c, &user, &ci, &ca, &transitive)
            .await
            .is_err(),
        "a redundant non-root ancestor cannot become an offered name"
    );
    assert_eq!(get_state(&c, &user, &ci, cg).await.unwrap(), before);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&c)
        .await
        .unwrap();
    assert_eq!(after, count);
    let mut obsolete = valid.envelope.clone();
    obsolete.parents.push(root.event().clone());
    let Action::GroupNameResolved { name } = &mut obsolete.action else {
        panic!("expected resolution")
    };
    *name = "original".into();
    let wire = causal_envelope::canonical_causal_envelope(&obsolete).unwrap();
    let obsolete = RenameBundle {
        envelope: obsolete,
        source_identity: valid.source_identity.clone(),
        root: valid.root.clone(),
        wire,
    };
    assert!(
        import_event(&c, &user, &ci, &ca, &obsolete).await.is_err(),
        "an obsolete ancestor cannot become an offered name"
    );
    assert_eq!(get_state(&c, &user, &ci, cg).await.unwrap(), before);
    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM causal_events")
        .fetch_one(&c)
        .await
        .unwrap();
    assert_eq!(after, count);
    assert!(
        matches!(import_event(&c, &user, &ci, &ca, &valid).await.unwrap(), RenameImportResult::Applied(NameState::Ready { name, .. }) if name == "alpha")
    );
}
