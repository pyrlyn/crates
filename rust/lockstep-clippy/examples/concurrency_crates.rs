// Paths resolve to the real crates: `crossbeam::scope` is defined in `crossbeam_utils`, and
// parking_lot's lock methods in `lock_api`.

use rayon::prelude::*;

fn main() {
    // rayon
    let v = vec![1, 2, 3];
    let total: i32 = v.par_iter().map(|x| x * 2).sum();
    let _: Vec<i32> = v.clone().into_par_iter().filter(|x| *x > 1).collect();
    let _ = v.iter().par_bridge().count();
    let _ = rayon::join(|| 1, || 2);
    rayon::scope(|s| s.spawn(|_| {}));
    rayon::spawn(|| {});

    // crossbeam
    let (tx, rx) = crossbeam::channel::unbounded::<i32>();
    tx.send(total).unwrap();
    let _ = rx.recv();
    let _ = crossbeam::scope(|s| {
        s.spawn(|_| {});
    });
    let _ = crossbeam::thread::scope(|_| {});
    let _ = crossbeam::sync::WaitGroup::new();
    crossbeam::select! {
        recv(rx) -> _ => {},
        default => {},
    }

    // parking_lot
    let lock = parking_lot::Mutex::new(0);
    *lock.lock() += 1;
    let _ = parking_lot::RwLock::new(0);

    // tokio::sync
    let _ = tokio::sync::Mutex::new(0);
    let _ = tokio::sync::RwLock::new(0);
    let _ = tokio::sync::Semaphore::new(1);
    let _ = tokio::sync::Barrier::new(1);
    let _ = tokio::sync::Notify::new();
    // Channels are not on the list.
    let _ = tokio::sync::oneshot::channel::<i32>();

    // spin
    let _ = spin::Mutex::new(0);
    let _ = spin::RwLock::new(0);

    // Sequential iteration is fine.
    let _: i32 = v.iter().sum();
}
