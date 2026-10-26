use std::sync::{Arc, Barrier, Condvar, Mutex, RwLock};
use std::thread;

fn main() {
    let shared = Arc::new(Mutex::new(0));
    let table: RwLock<Vec<i32>> = RwLock::new(Vec::new());
    let ready = (Mutex::new(false), Condvar::new());
    let barrier = Barrier::new(1);

    let handle = thread::spawn(|| 1);
    let _ = handle.join();

    thread::scope(|scope| {
        scope.spawn(|| 2);
    });

    let _ = thread::Builder::new().name("worker".into()).spawn(|| 3);

    let _ = std::thread::spawn(|| 4);

    // Not explicit threading: sleeping, atomics, `OnceLock`.
    thread::sleep(std::time::Duration::from_millis(0));
    let _ = std::sync::atomic::AtomicUsize::new(0);
    let _ = std::sync::OnceLock::<i32>::new();

    let _ = (shared, table, ready, barrier);
    takes_lock(&Mutex::new(1));
    allowed();
}

fn takes_lock(lock: &Mutex<i32>) -> i32 {
    *lock.lock().unwrap()
}

#[allow(lockstep_explicit_concurrency)]
fn allowed() {
    let _ = Mutex::new(0);
    let _ = thread::spawn(|| {});
}
