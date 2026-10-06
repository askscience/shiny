//! The command relay: the assistant (server side) → the FilmCraft window (browser side).
//!
//! The window runs inside the user's browser, so the engine it drives is not
//! reachable from Rust. Instead the window long-polls
//! `GET /api/filmcraft/next`, runs the request with `window.filmcraft`, and
//! posts the answer back to `POST /api/filmcraft/result`.
//!
//! Queues are **per user** (the route identity arrives in `x-shiny-user-id`),
//! so two signed-in accounts never see each other's commands, and a window only
//! ever drains its own queue.
//!
//! # Two structures per user
//!
//! A queued request and the oneshot waiting for its answer are kept apart:
//! `queue` holds what the window will run, `waiters` holds who to answer. The
//! split matters — the window takes a request out of `queue` and only runs it
//! later, so the sender has to survive in `waiters` until the answer arrives (or
//! the caller gives up).
//!
//! # Blocking note
//!
//! [`submit`] awaits the window's answer, and it is called from inside a
//! `bridged` tool — the plugin runtime is one serial worker shared by every
//! plugin (`rt.rs`). A call therefore holds that worker for its whole wait, so
//! the wait is bounded ([`DEFAULT_WAIT`], capped at [`MAX_WAIT`]) and callers
//! should use short control-channel commands here. Long work belongs in the
//! headless engine ([`crate::headless`]) or in the window itself.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::sync::oneshot;

/// How long a window heartbeat keeps it "online".
pub const WINDOW_TTL: Duration = Duration::from_secs(15);
/// Default ceiling of a `film_command` wait.
pub const DEFAULT_WAIT: Duration = Duration::from_secs(20);
/// Hard ceiling of a `film_command` wait.
pub const MAX_WAIT: Duration = Duration::from_secs(120);

type Answer = Result<Value, String>;

/// What the window will run.
struct Request {
    id: u64,
    method: String,
    params: Value,
}

struct User {
    next_id: u64,
    queue: VecDeque<Request>,
    waiters: HashMap<u64, oneshot::Sender<Answer>>,
    seen: Option<Instant>,
}

static USERS: OnceLock<Mutex<HashMap<String, User>>> = OnceLock::new();

fn users() -> &'static Mutex<HashMap<String, User>> {
    USERS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_users<T>(f: impl FnOnce(&mut HashMap<String, User>) -> T) -> T {
    // A poisoned relay must not take the window down with it: recover the map.
    let mut guard = users().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut guard)
}

fn entry<'a>(map: &'a mut HashMap<String, User>, user: &str) -> &'a mut User {
    map.entry(user.to_string())
        .or_insert_with(|| User {
            next_id: 1,
            queue: VecDeque::new(),
            waiters: HashMap::new(),
            seen: None,
        })
}

/// The window called `/next`: it is alive and polling.
pub fn heartbeat(user: &str) {
    with_users(|map| entry(map, user).seen = Some(Instant::now()));
}

/// Whether a window is polling for `user` right now.
pub fn window_online(user: &str) -> bool {
    with_users(|map| {
        map.get(user)
            .and_then(|u| u.seen)
            .is_some_and(|t| t.elapsed() < WINDOW_TTL)
    })
}

/// How many commands are waiting for a window.
pub fn queued(user: &str) -> usize {
    with_users(|map| map.get(user).map_or(0, |u| u.queue.len()))
}

/// Queue a request for the window and wait for its answer.
pub async fn submit(user: &str, method: &str, params: Value, wait: Duration) -> Answer {
    if !window_online(user) {
        return Err("no FilmCraft window is polling. Open the FilmCraft window first \
                    (action `show_plugin`, name \"filmcraft\"), then retry."
            .into());
    }
    let (tx, rx) = oneshot::channel();
    let id = with_users(|map| {
        let u = entry(map, user);
        let id = u.next_id;
        u.next_id += 1;
        u.queue.push_back(Request {
            id,
            method: method.to_string(),
            params,
        });
        u.waiters.insert(id, tx);
        id
    });

    match tokio::time::timeout(wait, rx).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(_)) => {
            // The sender is gone (a poisoned map drop, say).
            Err("the FilmCraft window closed before answering".into())
        }
        Err(_) => {
            // Give up: forget the waiter so the answer cannot resurrect it, and
            // drop the request if the window never picked it up.
            with_users(|map| {
                if let Some(u) = map.get_mut(user) {
                    u.waiters.remove(&id);
                    u.queue.retain(|r| r.id != id);
                }
            });
            Err(format!(
                "the FilmCraft window did not answer within {}s",
                wait.as_secs()
            ))
        }
    }
}

/// Pop the next request for a polling window, if any.
pub fn take(user: &str) -> Option<Value> {
    with_users(|map| {
        let u = entry(map, user);
        u.seen = Some(Instant::now());
        let r = u.queue.pop_front()?;
        Some(json!({ "id": r.id, "method": r.method, "params": r.params }))
    })
}

/// Deliver the window's answer to whoever is waiting for `id`.
///
/// Returns whether anyone was still listening — `false` means the caller timed
/// out and dropped the request, which is not an error.
pub fn complete(user: &str, id: u64, answer: Answer) -> bool {
    with_users(|map| {
        let Some(u) = map.get_mut(user) else {
            return false;
        };
        match u.waiters.remove(&id) {
            Some(tx) => tx.send(answer).is_ok(),
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip() {
        let user = "round-trip-user";
        heartbeat(user);
        assert!(window_online(user));
        let waiter = tokio::spawn(submit(
            user,
            "engine.execute",
            json!({ "command": "project.inspect" }),
            Duration::from_secs(5),
        ));
        // The window polls from another thread: the relay is synchronous, and
        // `submit` is waiting on this very task.
        let window = std::thread::spawn(move || {
            for _ in 0..200 {
                if let Some(req) = take(user) {
                    assert_eq!(req["method"], "engine.execute");
                    let id = req["id"].as_u64().expect("an id");
                    assert!(complete(user, id, Ok(json!({ "ok": 1 }))));
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("the relay never handed the request over");
        });
        assert_eq!(waiter.await.unwrap().unwrap(), json!({ "ok": 1 }));
        window.join().expect("the window thread finished");
        assert_eq!(queued(user), 0);
    }

    #[tokio::test]
    async fn an_error_answer_reaches_the_caller() {
        let user = "error-user";
        heartbeat(user);
        let waiter = tokio::spawn(submit(user, "engine.execute", json!({}), Duration::from_secs(5)));
        let window = std::thread::spawn(move || {
            for _ in 0..200 {
                if let Some(req) = take(user) {
                    let id = req["id"].as_u64().expect("an id");
                    complete(user, id, Err("no sequence".into()));
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("no request");
        });
        assert_eq!(waiter.await.unwrap().unwrap_err(), "no sequence");
        window.join().expect("the window thread finished");
    }

    #[tokio::test]
    async fn submit_without_window_is_an_explanatory_error() {
        let err = submit(
            "nobody-home",
            "engine.execute",
            json!({}),
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert!(err.contains("no FilmCraft window"), "{err}");
    }

    #[tokio::test]
    async fn timeout_drops_the_queued_command() {
        let user = "timeout-user";
        heartbeat(user);
        let err = submit(user, "engine.execute", json!({}), Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(err.contains("did not answer"), "{err}");
        assert_eq!(queued(user), 0, "a timed-out command must not linger");
        // A late answer for that id must not panic or resurrect anything.
        assert!(!complete(user, 1, Ok(json!({}))));
    }

    #[tokio::test]
    async fn queues_are_isolated_per_user() {
        heartbeat("iso-a");
        heartbeat("iso-b");
        let a = tokio::spawn(submit(
            "iso-a",
            "engine.execute",
            json!({ "command": "project.inspect" }),
            Duration::from_secs(5),
        ));
        // `submit` runs on its own task, so wait for the enqueue before
        // asserting on the queue.
        for _ in 0..200 {
            if queued("iso-a") == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(queued("iso-a"), 1);
        // Draining user b must never hand out user a's request.
        assert!(take("iso-b").is_none());
        assert_eq!(queued("iso-a"), 1, "draining b must not touch a's queue");
        let request = take("iso-a").expect("user a's request");
        let id = request["id"].as_u64().expect("an id");
        complete("iso-a", id, Ok(json!("done")));
        assert_eq!(a.await.unwrap().unwrap(), json!("done"));
    }
}