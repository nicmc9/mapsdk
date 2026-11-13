//! Тройной буфер для снимка кадра: продюсер (логика) пишет, консьюмер (рендер) читает
//! самое свежее. Без блокировок, без аллокаций в установившемся режиме.
//!
//! Реализация — крейт `triple_buffer` (lock-free, на атомиках). Здесь — тонкая обёртка
//! с нашими именами и счётчиками для статистики.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Создаёт тройной буфер. Все три слота заполняются значением `initial`.
pub fn triple_buffer<T: Clone + Send>(initial: &T) -> (Producer<T>, Consumer<T>) {
    let (input, output) = ::triple_buffer::triple_buffer(initial);
    let stats = Arc::new(Stats::default());
    (
        Producer {
            input,
            stats: Arc::clone(&stats),
        },
        Consumer { output, stats },
    )
}

#[derive(Debug, Default)]
struct Stats {
    published: AtomicU64,
    taken: AtomicU64,
}

/// Сторона логики.
// [Rust] Debug реализован вручную: у `triple_buffer::Input` его нет.
pub struct Producer<T: Send> {
    input: ::triple_buffer::Input<T>,
    stats: Arc<Stats>,
}

impl<T: Send> Producer<T> {
    /// Слот для записи. В нём лежат данные, опубликованные несколько кадров назад:
    /// очищайте коллекции через `clear()` — память переиспользуется.
    pub fn write(&mut self) -> &mut T {
        self.input.input_buffer_mut()
    }

    /// Публикует записанное. Если консьюмер не забрал предыдущий снимок, тот теряется.
    pub fn publish(&mut self) {
        self.input.publish();
        self.stats.published.fetch_add(1, Ordering::Relaxed);
    }
}

impl<T: Send> std::fmt::Debug for Producer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Producer").finish_non_exhaustive()
    }
}

/// Сторона рендера.
pub struct Consumer<T: Send> {
    output: ::triple_buffer::Output<T>,
    stats: Arc<Stats>,
}

impl<T: Send> Consumer<T> {
    /// Забирает самый свежий снимок. `true` — есть новый с прошлого вызова.
    pub fn take(&mut self) -> bool {
        let updated = self.output.update();
        if updated {
            self.stats.taken.fetch_add(1, Ordering::Relaxed);
        }
        updated
    }

    /// Текущий снимок (последний забранный).
    pub fn current(&self) -> &T {
        self.output.output_buffer()
    }

    /// Обнулить счётчик пропусков: всё опубликованное до этого момента считается увиденным.
    /// Вызывается, когда консьюмер начинает работать после простоя — пропуски за время
    /// простоя (окно скрыто, приложение в фоне) не должны попадать в статистику.
    pub fn resync(&mut self) {
        let published = self.stats.published.load(Ordering::Relaxed);
        self.stats.taken.store(published, Ordering::Relaxed);
    }

    /// Сколько опубликованных снимков консьюмер не увидел (перезаписаны более новыми).
    pub fn skipped(&self) -> u64 {
        let published = self.stats.published.load(Ordering::Relaxed);
        let taken = self.stats.taken.load(Ordering::Relaxed);
        published.saturating_sub(taken)
    }
}

impl<T: Send> std::fmt::Debug for Consumer<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Consumer").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct Snapshot {
        frame: u64,
        items: Vec<u64>,
    }

    #[test]
    fn consumer_sees_latest() {
        let (mut producer, mut consumer) = triple_buffer(&Snapshot::default());
        for frame in 1..=3 {
            producer.write().frame = frame;
            producer.publish();
        }
        assert!(consumer.take());
        assert_eq!(consumer.current().frame, 3);
        assert!(!consumer.take(), "нового снимка нет");
        assert_eq!(consumer.skipped(), 2);
    }

    #[test]
    fn slots_reuse_memory() {
        let (mut producer, mut consumer) = triple_buffer(&Snapshot::default());
        // Прогрев: каждый из трёх слотов один раз выделяет память
        for frame in 0..6 {
            let slot = producer.write();
            slot.items.clear();
            slot.items.extend(0..1000);
            slot.frame = frame;
            producer.publish();
            consumer.take();
        }
        // Дальше ёмкость уже достаточна — указатели на данные не меняются
        let mut seen = std::collections::HashSet::new();
        for frame in 6..100 {
            let slot = producer.write();
            slot.items.clear();
            slot.items.extend(0..1000);
            slot.frame = frame;
            seen.insert(slot.items.as_ptr() as usize);
            producer.publish();
            consumer.take();
        }
        assert!(
            seen.len() <= 3,
            "аллокации в установившемся режиме: {}",
            seen.len()
        );
    }

    #[test]
    fn concurrent_snapshots_are_never_torn() {
        let (mut producer, mut consumer) = triple_buffer(&Snapshot::default());
        std::thread::scope(|scope| {
            scope.spawn(move || {
                for frame in 1..=100_000u64 {
                    let slot = producer.write();
                    slot.frame = frame;
                    slot.items.clear();
                    slot.items.extend([frame; 8]);
                    producer.publish();
                }
            });
            let mut last = 0;
            while last < 100_000 {
                if consumer.take() {
                    let snapshot = consumer.current();
                    // Снимок целостный: все поля из одного кадра
                    assert!(snapshot.items.iter().all(|&x| x == snapshot.frame));
                    // И кадры не идут назад
                    assert!(snapshot.frame >= last);
                    last = snapshot.frame;
                }
            }
        });
    }
}
