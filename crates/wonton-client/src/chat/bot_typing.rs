use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};

use futures_util::StreamExt;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use wonton_matrix::MatrixSession;
use wonton_protocol::{ChatViewModel, RoomMembership};

use super::bots::Bots;
use super::lock;

#[derive(Default)]
struct Subscriptions(Vec<JoinHandle<()>>);

impl Drop for Subscriptions {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}

pub(super) async fn follow(
    bots: Weak<Bots>,
    mut session: watch::Receiver<Option<Arc<MatrixSession>>>,
    mut view: watch::Receiver<ChatViewModel>,
    mut foreground: watch::Receiver<bool>,
) {
    let Some(link) = bots.upgrade() else { return };
    let mut home = link.home.subscribe();
    drop(link);
    let mut epoch = u64::MAX;
    let mut profile_task = Subscriptions::default();
    let mut typing_tasks = Subscriptions::default();
    let mut previous = Vec::new();
    loop {
        let Some(link) = bots.upgrade() else { return };
        let current = session.borrow_and_update().clone();
        let next_epoch = link.epoch.load(Ordering::SeqCst);
        if next_epoch != epoch {
            for task in profile_task.0.drain(..) {
                task.abort();
            }
            if let Some(current) = &current {
                profile_task
                    .0
                    .push(profile_changes(bots.clone(), current.clone()));
            }
        }
        let mut targets = Vec::new();
        if current.is_some() && link.can_poll() {
            let view = view.borrow_and_update();
            for room in &view.rooms {
                if room.membership != RoomMembership::Joined {
                    continue;
                }
                for bot in &view.bots {
                    if room.direct_user_id.as_deref() == Some(&bot.user_id) {
                        targets.push((room.room_id.clone(), bot.user_id.clone()));
                    }
                }
            }
        }
        targets.sort();
        if next_epoch != epoch || targets != previous {
            for task in typing_tasks.0.drain(..) {
                task.abort();
            }
            {
                let mut local = lock(&link.local);
                for record in local.records.values_mut() {
                    if next_epoch != epoch {
                        record.typing.clear();
                    }
                    record.typing.retain(|room, _| {
                        targets
                            .iter()
                            .any(|(id, user)| id == room && user == &record.user_id)
                    });
                }
            }
            if let Some(current) = current {
                typing_tasks = subscribe_typing(&bots, &current, &targets, next_epoch);
            }
            link.update_activity();
            link.changed.notify_one();
            previous = targets;
        }
        epoch = next_epoch;
        drop(link);
        tokio::select! {
            result = session.changed() => if result.is_err() { return },
            result = view.changed() => if result.is_err() { return },
            result = home.changed() => if result.is_err() { return },
            result = foreground.changed() => if result.is_err() { return },
        }
    }
}

fn set_typing(bots: &Bots, room: &str, users: &[wonton_matrix::OwnedUserId]) {
    // A room's typing list is a complete snapshot, not an incremental add/remove event.
    let direct = bots
        .view
        .borrow()
        .rooms
        .iter()
        .find(|row| row.room_id == room)
        .and_then(|row| row.direct_user_id.clone());
    let Some(direct) = direct else { return };
    let mut local = lock(&bots.local);
    for record in local
        .records
        .values_mut()
        .filter(|record| record.user_id == direct)
    {
        record.typing.insert(
            room.to_owned(),
            users.iter().any(|user| user.as_str() == direct),
        );
    }
    drop(local);
    bots.update_activity();
    bots.changed.notify_one();
}

fn profile_changes(bots: Weak<Bots>, session: Arc<MatrixSession>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut changes = std::pin::pin!(session.profile_changes());
        while let Some(user) = changes.next().await {
            let Some(link) = bots.upgrade() else { return };
            if !link
                .session
                .borrow()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &session))
            {
                return;
            }
            let ids: Vec<_> = lock(&link.local)
                .records
                .iter()
                .filter(|(_, record)| record.user_id == user.as_str())
                .map(|(id, _)| id.clone())
                .collect();
            for id in ids {
                link.refresh(&id);
            }
        }
    })
}

fn subscribe_typing(
    bots: &Weak<Bots>,
    session: &MatrixSession,
    targets: &[(String, String)],
    epoch: u64,
) -> Subscriptions {
    let mut tasks = Subscriptions::default();
    for (room, _) in targets {
        let Ok(room) = room.parse::<wonton_matrix::OwnedRoomId>() else {
            continue;
        };
        let Ok(stream) = session.typing_notifications(&room) else {
            continue;
        };
        let bots = bots.clone();
        let room = room.to_string();
        tasks.0.push(tokio::spawn(async move {
            let mut stream = std::pin::pin!(stream);
            while let Some(users) = stream.next().await {
                let Some(link) = bots.upgrade() else { return };
                if link.can_poll() && link.epoch.load(Ordering::SeqCst) == epoch {
                    set_typing(&link, &room, &users);
                }
            }
        }));
    }
    tasks
}
