//! Одноразовый ответ на синхронный запрос — аналог `std::promise` / `std::future`.

use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};

/// Создаёт пару «отправить ответ» / «дождаться ответа».
pub fn reply<T>() -> (ReplySender<T>, ReplyReceiver<T>) {
    let (tx, rx) = crossbeam_channel::bounded(1);
    (ReplySender { tx }, ReplyReceiver { rx })
}

#[derive(Debug)]
pub struct ReplySender<T> {
    tx: Sender<T>,
}

impl<T> ReplySender<T> {
    /// Отправляет ответ. [Rust] `self` без `&` — метод забирает отправитель,
    /// второй раз ответить невозможно, это проверяет компилятор.
    pub fn send(self, value: T) {
        // Ждущий мог уйти по таймауту — это не ошибка отвечающего
        let _ = self.tx.send(value);
    }
}

#[derive(Debug)]
pub struct ReplyReceiver<T> {
    rx: Receiver<T>,
}

impl<T> ReplyReceiver<T> {
    /// Ждёт ответ не дольше `timeout`. `None` — таймаут или отвечающий уничтожен без ответа.
    pub fn wait(self, timeout: Duration) -> Option<T> {
        self.rx.recv_timeout(timeout).ok()
    }
}
