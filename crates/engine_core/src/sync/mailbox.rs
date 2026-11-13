//! Почтовый ящик потока: неограниченная очередь «много отправителей → один получатель».

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crossbeam_channel::{Receiver, Sender};

/// Порог длины очереди, после которого пишем предупреждение: получатель не успевает разбирать.
const WARN_LEN: usize = 1024;

/// Создаёт почтовый ящик. `name` используется в логах.
///
/// ```ignore
/// let (tx, rx) = mailbox::<RenderCommand>("render");
/// tx.send(RenderCommand::Resize { .. });   // из любого потока
/// for cmd in rx.drain() { /* ... */ }       // в потоке-владельце
/// ```
pub fn mailbox<T>(name: &'static str) -> (MailboxSender<T>, Mailbox<T>) {
    // [Rust] crossbeam-channel: lock-free очередь. `unbounded` никогда не блокирует отправителя,
    // поэтому два потока, пишущие друг другу, не попадут в deadlock (см. архитектуру, 4.2).
    let (tx, rx) = crossbeam_channel::unbounded();
    let warned = Arc::new(AtomicUsize::new(WARN_LEN));
    (
        MailboxSender {
            tx,
            name,
            warn_at: warned,
        },
        Mailbox { rx, name },
    )
}

/// Отправляющая сторона. Дёшево клонируется: отправителей может быть сколько угодно.
// [Rust] derive(Clone) здесь не подходит: он потребовал бы `T: Clone`, хотя копируется
// только канал, а не сообщения. Поэтому Clone реализован вручную ниже.
#[derive(Debug)]
pub struct MailboxSender<T> {
    tx: Sender<T>,
    name: &'static str,
    /// Длина, при которой будет следующее предупреждение (удваивается, чтобы не спамить).
    warn_at: Arc<AtomicUsize>,
}

impl<T> Clone for MailboxSender<T> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            name: self.name,
            warn_at: Arc::clone(&self.warn_at),
        }
    }
}

impl<T> MailboxSender<T> {
    /// Отправляет сообщение. Сообщение перемещается в очередь — копирования данных нет.
    /// Возвращает `false`, если получатель уже уничтожен (поток завершился).
    pub fn send(&self, message: T) -> bool {
        if self.tx.send(message).is_err() {
            return false;
        }
        let len = self.tx.len();
        let warn_at = self.warn_at.load(Ordering::Relaxed);
        if len >= warn_at {
            self.warn_at.store(warn_at * 2, Ordering::Relaxed);
            log::warn!("Очередь «{}» растёт: {len} сообщений", self.name);
        }
        true
    }

    /// Сколько сообщений ждут разбора.
    pub fn len(&self) -> usize {
        self.tx.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tx.is_empty()
    }
}

/// Принимающая сторона. Владеет ей ровно один поток.
#[derive(Debug)]
pub struct Mailbox<T> {
    rx: Receiver<T>,
    name: &'static str,
}

impl<T> Mailbox<T> {
    /// Забирает все накопившиеся сообщения, не блокируясь.
    // [Rust] Возвращается итератор: сообщения достаются по одному в цикле `for`,
    // промежуточный Vec не создаётся.
    pub fn drain(&self) -> impl Iterator<Item = T> + '_ {
        self.rx.try_iter()
    }

    /// Ждёт одно сообщение не дольше `timeout`. `None` — таймаут или все отправители уничтожены.
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<T> {
        // [Rust] `.ok()` превращает Result в Option, отбрасывая ошибку
        self.rx.recv_timeout(timeout).ok()
    }

    /// Ждёт одно сообщение. `None` — все отправители уничтожены, сообщений больше не будет.
    pub fn recv(&self) -> Option<T> {
        self.rx.recv().ok()
    }

    /// Одно сообщение без ожидания.
    pub fn try_recv(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_order_from_many_threads() {
        let (tx, rx) = mailbox::<(usize, usize)>("test");
        // [Rust] `thread::scope` — потоки гарантированно завершатся до выхода из блока,
        // поэтому им можно отдавать ссылки на локальные переменные.
        std::thread::scope(|scope| {
            for sender in 0..4 {
                let tx = tx.clone();
                scope.spawn(move || {
                    for i in 0..1000 {
                        tx.send((sender, i));
                    }
                });
            }
        });
        let mut last = [None::<usize>; 4];
        let mut total = 0;
        for (sender, i) in rx.drain() {
            // Порядок сообщений одного отправителя сохраняется
            assert!(last[sender].is_none_or(|prev| prev + 1 == i));
            last[sender] = Some(i);
            total += 1;
        }
        assert_eq!(total, 4000);
    }

    #[test]
    fn send_fails_after_receiver_dropped() {
        let (tx, rx) = mailbox::<u32>("test");
        drop(rx);
        assert!(!tx.send(1));
    }
}
