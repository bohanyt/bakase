//! Satu giliran chat aktif per percakapan. Catatannya hanya di memori, jadi hilang sendiri kalau
//! proses berhenti. Saat pindah slot, dua slot sempat bisa menjalankan percakapan yang sama.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::http::StatusCode;
use tokio::sync::watch;

use crate::error::ApiError;

/// Tombol Hentikan baru terlihat server saat penulisan berikutnya ke browser, paling lama satu
/// jarak keepalive SSE (15 detik).
const WAIT: Duration = Duration::from_secs(20);

type Key = (i64, String);

struct Running {
    turn_id: Option<String>,
    cancel: watch::Sender<bool>,
    /// Tertutup begitu gilirannya selesai.
    done: watch::Receiver<()>,
}

#[derive(Clone, Default)]
pub struct ActiveTurns(Arc<Mutex<HashMap<Key, Running>>>);

/// Dipegang task giliran. Dilepas saat di-drop, termasuk kalau task panik.
pub struct Active {
    turns: ActiveTurns,
    key: Key,
    cancel: watch::Receiver<bool>,
    _done: watch::Sender<()>,
}

impl Active {
    /// Selesai kalau pengirim yang sama sudah mengirim pesan berikutnya.
    pub fn superseded(&self) -> impl Future<Output = ()> + Send + use<> {
        let mut cancel = self.cancel.clone();
        async move {
            if cancel.wait_for(|cancelled| *cancelled).await.is_err() {
                std::future::pending::<()>().await;
            }
        }
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        if let Ok(mut turns) = self.turns.0.lock() {
            turns.remove(&self.key);
        }
    }
}

impl ActiveTurns {
    /// Menunggu giliran lain di percakapan ini selesai. `previous` adalah giliran terakhir dari
    /// pengirim yang sama: ia tidak lagi menunggu jawabannya, jadi giliran itu dihentikan.
    pub async fn claim(
        &self,
        user_id: i64,
        conversation_id: &str,
        turn_id: Option<&str>,
        previous: Option<&str>,
    ) -> Result<Active, ApiError> {
        let key = (user_id, conversation_id.to_string());
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let mut done = {
                let mut turns = self
                    .0
                    .lock()
                    .map_err(|_| anyhow::anyhow!("daftar giliran aktif rusak"))?;
                match turns.get(&key) {
                    None => {
                        let (cancel, cancelled) = watch::channel(false);
                        let (finished, done) = watch::channel(());
                        let turn_id = turn_id.map(str::to_string);
                        turns.insert(
                            key.clone(),
                            Running {
                                turn_id,
                                cancel,
                                done,
                            },
                        );
                        return Ok(Active {
                            turns: self.clone(),
                            key,
                            cancel: cancelled,
                            _done: finished,
                        });
                    }
                    Some(running) => {
                        if previous.is_some() && running.turn_id.as_deref() == previous {
                            running.cancel.send_replace(true);
                        }
                        running.done.clone()
                    }
                }
            };
            if tokio::time::timeout_at(deadline, done.changed())
                .await
                .is_err()
            {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "Masih ada jawaban yang berjalan di obrolan ini, mungkin dari tab lain. Tunggu sampai selesai, lalu kirim lagi.",
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn another_tab_waits_then_is_refused() {
        let turns = ActiveTurns::default();
        let first = turns.claim(1, "a", Some("satu"), None).await.unwrap();
        // Percakapan lain dan user lain tidak ikut tertahan.
        let _other = turns.claim(1, "b", Some("dua"), None).await.unwrap();
        let _user = turns.claim(2, "a", Some("tiga"), None).await.unwrap();
        assert!(turns.claim(1, "a", Some("empat"), None).await.is_err());
        assert!(
            turns
                .claim(1, "a", Some("empat"), Some("lain"))
                .await
                .is_err()
        );
        assert!(!*first.cancel.borrow(), "tab lain tidak menghentikannya");

        let waiting = tokio::spawn({
            let turns = turns.clone();
            async move { turns.claim(1, "a", Some("lima"), None).await.is_ok() }
        });
        tokio::task::yield_now().await;
        drop(first);
        assert!(waiting.await.unwrap());
        assert_eq!(turns.0.lock().unwrap().len(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_next_message_from_the_same_sender_stops_the_old_turn() {
        let turns = ActiveTurns::default();
        let first = turns.claim(1, "a", Some("satu"), None).await.unwrap();
        let task = tokio::spawn(async move {
            first.superseded().await;
            drop(first);
        });
        let started = tokio::time::Instant::now();
        let next = turns.claim(1, "a", Some("dua"), Some("satu")).await;
        assert!(next.is_ok());
        assert!(started.elapsed() < Duration::from_secs(1));
        task.await.unwrap();
    }
}
