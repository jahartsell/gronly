//! Example library for gronly-atomics

use gronly_atomics::sync::atomic::AtomicUsize;
use gronly_atomics::sync::atomic::Ordering;

pub fn increment(value: &AtomicUsize) {
    // Buggy
    // let v = value.load(Ordering::Acquire);
    // value.store(v + 1, Ordering::Release);
    value.fetch_add(1, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    use gronly_atomics::sync::Arc;
    use gronly_atomics::{modeled_test, thread};

    #[modeled_test]
    fn increment_works() {
        let num = Arc::new(AtomicUsize::new(0));

        let threads: Vec<_> = (0..2)
            .map(|_| {
                let num = Arc::clone(&num);
                thread::spawn(move || increment(&num))
            })
            .collect();

        for t in threads {
            t.join().unwrap();
        }

        assert_eq!(num.load(Ordering::Relaxed), 2);
    }
}
