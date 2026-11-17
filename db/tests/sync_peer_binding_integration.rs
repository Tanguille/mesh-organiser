//! Peer bindings pin caller-supplied identities; they do not authenticate remote peers.
mod common;

use db::{
    db_context::DbContext,
    model::user::User,
    random_hex_32, sync_peer,
    user_db::{self, SyncIdentity},
};

async fn identity(db: &DbContext, user: &User) -> SyncIdentity {
    user_db::get_sync_identity(db, user.id)
        .await
        .unwrap()
        .unwrap()
}

fn remote() -> SyncIdentity {
    SyncIdentity {
        instance_id: random_hex_32(),
        account_id: random_hex_32(),
    }
}

async fn added_user(db: &DbContext, name: &str) -> User {
    let id = user_db::add_user(db, name, &format!("{name}@example.test"), &random_hex_32())
        .await
        .unwrap();
    user_db::get_user_by_id(db, id).await.unwrap().unwrap()
}

#[tokio::test]
async fn repeated_binding_is_idempotent_and_distinct_pairs_are_isolated() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let other = added_user(&db, "other").await;
    let local = identity(&db, &owner).await;
    let other_local = identity(&db, &other).await;
    let first = remote();
    let second = SyncIdentity {
        instance_id: first.instance_id.clone(),
        account_id: random_hex_32(),
    };
    let third = SyncIdentity {
        instance_id: random_hex_32(),
        account_id: first.account_id.clone(),
    };
    let binding = sync_peer::bind_peer(&db, &owner, &local, &first)
        .await
        .unwrap();
    assert_eq!(binding.local, local);
    assert_eq!(binding.remote, first);
    sync_peer::bind_peer(&db, &owner, &local, &first)
        .await
        .unwrap();
    sync_peer::bind_peer(&db, &owner, &local, &second)
        .await
        .unwrap();
    sync_peer::bind_peer(&db, &owner, &local, &third)
        .await
        .unwrap();
    sync_peer::bind_peer(&db, &other, &other_local, &first)
        .await
        .unwrap();
    let peers = sync_peer::list_peers(&db, &owner, &local).await.unwrap();
    assert_eq!(peers.len(), 3);
    for expected in [&first, &second, &third] {
        assert_eq!(peers.iter().filter(|p| &p.remote == expected).count(), 1);
    }
    let other_peers = sync_peer::list_peers(&db, &other, &other_local)
        .await
        .unwrap();
    assert_eq!(other_peers.len(), 1);
    assert_eq!(other_peers[0].remote, first);
}

#[tokio::test]
async fn invalid_and_stale_identities_cannot_bind_or_list() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let local = identity(&db, &owner).await;
    let peer = remote();
    for bad in [
        SyncIdentity {
            instance_id: random_hex_32(),
            ..local.clone()
        },
        SyncIdentity {
            account_id: random_hex_32(),
            ..local.clone()
        },
    ] {
        assert!(
            sync_peer::bind_peer(&db, &owner, &bad, &peer)
                .await
                .is_err()
        );
        assert!(sync_peer::list_peers(&db, &owner, &bad).await.is_err());
    }
    for bad in [
        "",
        "not-hex",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "0123456789abcdef0123456789abcde",
    ] {
        assert!(
            sync_peer::bind_peer(
                &db,
                &owner,
                &local,
                &SyncIdentity {
                    instance_id: bad.into(),
                    ..peer.clone()
                }
            )
            .await
            .is_err()
        );
        assert!(
            sync_peer::bind_peer(
                &db,
                &owner,
                &local,
                &SyncIdentity {
                    account_id: bad.into(),
                    ..peer.clone()
                }
            )
            .await
            .is_err()
        );
    }
    assert_eq!(
        sync_peer::list_peers(&db, &owner, &local).await.unwrap(),
        []
    );
}

#[tokio::test]
async fn replacement_account_cannot_inherit_old_binding_or_use_old_pin() {
    let (_dir, db) = common::test_db().await;
    let owner = added_user(&db, "old").await;
    let local = identity(&db, &owner).await;
    let peer = remote();
    sync_peer::bind_peer(&db, &owner, &local, &peer)
        .await
        .unwrap();
    user_db::delete_user(&db, owner.id).await.unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'new', 'new@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(owner.id).execute(&db).await.unwrap();
    let replacement = user_db::get_user_by_id(&db, owner.id)
        .await
        .unwrap()
        .unwrap();
    let replacement_local = identity(&db, &replacement).await;
    assert_ne!(local.account_id, replacement_local.account_id);
    assert!(
        sync_peer::bind_peer(&db, &owner, &local, &peer)
            .await
            .is_err()
    );
    assert!(sync_peer::list_peers(&db, &owner, &local).await.is_err());
    assert_eq!(
        sync_peer::list_peers(&db, &replacement, &replacement_local)
            .await
            .unwrap(),
        []
    );
}

#[tokio::test]
async fn sync_url_and_token_changes_do_not_rebind_peer() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let local = identity(&db, &owner).await;
    let peer = remote();
    sync_peer::bind_peer(&db, &owner, &local, &peer)
        .await
        .unwrap();
    user_db::set_user_sync_token(
        &db,
        owner.id,
        &random_hex_32(),
        "https://new.example.test",
        false,
    )
    .await
    .unwrap();
    let peers = sync_peer::list_peers(&db, &owner, &local).await.unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].remote, peer);
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_binds_are_idempotent_without_losing_distinct_pairs() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let local = identity(&db, &owner).await;
    let first = remote();
    let second = remote();
    let (a, b, c) = tokio::join!(
        sync_peer::bind_peer(&db, &owner, &local, &first),
        sync_peer::bind_peer(&db, &owner, &local, &first),
        sync_peer::bind_peer(&db, &owner, &local, &second),
    );
    a.unwrap();
    b.unwrap();
    c.unwrap();
    let peers = sync_peer::list_peers(&db, &owner, &local).await.unwrap();
    assert_eq!(peers.len(), 2);
    assert!(peers.iter().any(|p| p.remote == first));
    assert!(peers.iter().any(|p| p.remote == second));
}

#[tokio::test]
async fn failed_insert_rolls_back_and_preserves_existing_binding() {
    let (_dir, db) = common::test_db().await;
    let owner = User::default();
    let local = identity(&db, &owner).await;
    let first = remote();
    let second = remote();
    sync_peer::bind_peer(&db, &owner, &local, &first)
        .await
        .unwrap();
    sqlx::query("CREATE TRIGGER reject_peer_binding BEFORE INSERT ON sync_peer_bindings BEGIN SELECT RAISE(ABORT, 'injected failure'); END").execute(&db).await.unwrap();
    assert!(
        sync_peer::bind_peer(&db, &owner, &local, &second)
            .await
            .is_err()
    );
    let remaining = sync_peer::list_peers(&db, &owner, &local).await.unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].remote, first);
    sqlx::query("DROP TRIGGER reject_peer_binding")
        .execute(&db)
        .await
        .unwrap();
    sync_peer::bind_peer(&db, &owner, &local, &second)
        .await
        .unwrap();
    assert_eq!(
        sync_peer::list_peers(&db, &owner, &local)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn replacement_under_write_lock_cannot_accept_stale_binding() {
    let (_dir, db) = common::test_db().await;
    let owner = added_user(&db, "racing").await;
    let local = identity(&db, &owner).await;
    let peer = remote();
    let mut tx = db.begin_with("BEGIN IMMEDIATE").await.unwrap();
    sqlx::query("DELETE FROM users WHERE user_id = ?")
        .bind(owner.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (user_id, user_name, user_email, user_password_hash, user_created_at) VALUES (?, 'new', 'raced@example.test', 'unused', '2020-01-01T00:00:00Z')").bind(owner.id).execute(&mut *tx).await.unwrap();
    let task = tokio::spawn({
        let (db, owner, local, peer) = (db.clone(), owner.clone(), local.clone(), peer.clone());
        async move { sync_peer::bind_peer(&db, &owner, &local, &peer).await }
    });
    tx.commit().await.unwrap();
    assert!(task.await.unwrap().is_err());
    let new_local = identity(&db, &owner).await;
    assert_ne!(local.account_id, new_local.account_id);
    assert_eq!(
        sync_peer::list_peers(&db, &owner, &new_local)
            .await
            .unwrap(),
        []
    );
}

#[tokio::test]
async fn malformed_stored_peer_identity_fails_closed() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let local = identity(&db, &user).await;
    let peer = remote();
    sync_peer::bind_peer(&db, &user, &local, &peer)
        .await
        .unwrap();
    sqlx::query("UPDATE sync_peer_bindings SET remote_account_id = 'invalid' WHERE user_id = ?")
        .bind(user.id)
        .execute(&db)
        .await
        .unwrap();
    assert!(sync_peer::list_peers(&db, &user, &local).await.is_err());
}

#[tokio::test]
async fn deleting_local_account_identity_cascades_bindings() {
    let (_dir, db) = common::test_db().await;
    let user = User::default();
    let local = identity(&db, &user).await;
    sync_peer::bind_peer(&db, &user, &local, &remote())
        .await
        .unwrap();
    sqlx::query("DELETE FROM sync_accounts WHERE user_id = ?")
        .bind(user.id)
        .execute(&db)
        .await
        .unwrap();
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_peer_bindings WHERE user_id = ?")
        .bind(user.id)
        .fetch_one(&db)
        .await
        .unwrap();
    assert_eq!(rows, 0);
    assert!(sync_peer::list_peers(&db, &user, &local).await.is_err());
}
