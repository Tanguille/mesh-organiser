//! Local-only event volume must not expand portable identity-claim lookups.
mod common;

#[tokio::test]
async fn portable_identity_claim_indexes_exclude_hot_local_history() {
    let (_dir, db) = common::test_db().await;
    for name in ["causal_event_global_claim", "causal_event_birth_claim"] {
        let index: String =
            sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='index' AND name=?")
                .bind(name)
                .fetch_one(&db)
                .await
                .unwrap();
        let normalized = index.to_lowercase();
        assert!(
            normalized.contains("where")
                && normalized.contains("encoding")
                && normalized.contains("portable_v2"),
            "{name} must exclude local-only journal rows: {index}"
        );
    }
    // EXPLAIN the claim lookups as written in production: extract each portable_v2
    // claim subquery from the source so the plan cannot drift from the real SQL.
    let source = include_str!("../src/causal_store.rs");
    let queries = source
        .match_indices("SELECT 1 FROM causal_events JOIN causal_event_receipts")
        .map(|(start, _)| {
            let line = source[start..].lines().next().unwrap();
            line.trim_end_matches([',', '"']).trim_end_matches(')')
        });
    let expected = [
        "causal_event_global_claim",
        "causal_event_birth_claim",
        "causal_event_global_claim",
    ];
    let claim_queries: Vec<&str> = queries
        .filter(|query| query.contains("encoding = 'portable_v2'"))
        .collect();
    assert_eq!(
        claim_queries.len(),
        expected.len(),
        "every received global, birth and unknown claim lookup must restrict the encoding: {claim_queries:?}"
    );
    for (query, index) in claim_queries.into_iter().zip(expected) {
        let mut statement =
            sqlx::query_as(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {query}")));
        for _ in 0..query.matches('?').count() {
            statement = statement.bind("group");
        }
        let plan: Vec<(i64, i64, i64, String)> = statement.fetch_all(&db).await.unwrap();
        assert!(
            plan.iter().any(|row| row.3.contains(index)),
            "claim lookup should use {index}: {query}: {plan:?}"
        );
    }
}
