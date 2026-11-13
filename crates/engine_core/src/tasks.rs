//! Асинхронные задачи (C-08). Реализация — `bevy_tasks`, наружу — только этот модуль.
//!
//! Два пула:
//! - **IO** — ожидание файлов и сети. Потоков немного: они в основном ждут.
//! - **Compute** — тяжёлые вычисления вне кадра: декодирование PNG, разбор glTF.
//!
//! Третий пул — для параллельных систем ECS — получает остальные ядра.
//!
//! На Web потоков нет: задачи выполняются в главном потоке через цикл событий браузера
//! (`spawn_local`), между кадрами.
//!
//! ```ignore
//! let mut task = tasks::spawn_io(async move { std::fs::read(path) });
//! // ... в системе, каждый кадр:
//! if let Some(result) = tasks::poll(&mut task) { /* готово */ }
//! ```
//!
//! [Rust] `async`-блок — это не поток и не запущенная работа, а описание работы (`Future`).
//! Выполнять её начинает исполнитель (пул), которому её передали. Аналог — лямбда,
//! отправленная в thread pool, но умеющая приостанавливаться в точках `.await`.

use std::future::Future;

use bevy_tasks::{AsyncComputeTaskPool, ComputeTaskPool, IoTaskPool, TaskPool};

/// Запущенная задача. Опрашивайте через [`poll`]. Уничтожение задачи отменяет её.
pub use bevy_tasks::Task;

/// `Send` на native и пустое ограничение на Web, где объекты (например, JS-промисы)
/// нельзя передавать между потоками, но и потоков нет.
pub use bevy_tasks::{BoxedFuture, ConditionalSend, ConditionalSendFuture};

/// Выполнить future до конца, блокируя текущий поток. На Web не использовать.
pub use bevy_tasks::block_on;

/// Создаёт пулы. Вызывается из `App::new`; повторный вызов ничего не делает.
pub(crate) fn init_pools() {
    let cores = bevy_tasks::available_parallelism();
    // Четверть ядер на IO и на вычисления, но не меньше одного потока
    let quarter = (cores / 4).max(1);
    IoTaskPool::get_or_init(|| pool("io", quarter));
    AsyncComputeTaskPool::get_or_init(|| pool("compute", quarter));
    // Пул систем ECS создаём сами: иначе bevy_ecs создаст его лениво при первом кадре,
    // а tick_main_thread ожидает, что все три пула уже есть
    ComputeTaskPool::get_or_init(|| pool("ecs", cores.saturating_sub(2 * quarter).max(1)));
}

#[cfg(not(target_arch = "wasm32"))]
fn pool(name: &str, threads: usize) -> TaskPool {
    bevy_tasks::TaskPoolBuilder::new()
        .num_threads(threads)
        .thread_name(name.to_owned())
        .build()
}

#[cfg(target_arch = "wasm32")]
fn pool(_name: &str, _threads: usize) -> TaskPool {
    TaskPool::new()
}

/// Запускает задачу ввода-вывода.
pub fn spawn_io<T>(future: impl Future<Output = T> + ConditionalSend + 'static) -> Task<T>
where
    T: ConditionalSend + 'static,
{
    IoTaskPool::get().spawn(future)
}

/// Запускает тяжёлую вычислительную задачу.
pub fn spawn_compute<T>(future: impl Future<Output = T> + ConditionalSend + 'static) -> Task<T>
where
    T: ConditionalSend + 'static,
{
    AsyncComputeTaskPool::get().spawn(future)
}

/// Проверяет, готова ли задача, не блокируя. `Some` — готова, результат забран.
pub fn poll<T>(task: &mut Task<T>) -> Option<T> {
    bevy_tasks::futures::check_ready(task)
}

/// На native задачи, запущенные из главного потока через `spawn_local`, исполняются
/// только когда главный поток их «прокручивает». Вызывается раз в кадр.
pub(crate) fn tick_main_thread() {
    #[cfg(not(target_arch = "wasm32"))]
    bevy_tasks::tick_global_task_pools_on_main_thread();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_and_compute_tasks_complete() {
        init_pools();
        let mut io = spawn_io(async { 2 + 2 });
        let mut compute = spawn_compute(async { "готово" });
        // В тесте можно ждать блокирующе; в кадре — только poll
        assert_eq!(block_on(&mut io), 4);
        assert_eq!(block_on(&mut compute), "готово");
        tick_main_thread();
    }
}
