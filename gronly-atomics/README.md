# gronly-atomics

Re-exports atomics from different backends depending on the configuration. This is intended to
simplify testing using modeled atomics from [Loom] or [Shuttle]. This crate re-exports
**everything** from a particular backend. The selection order is as follows: cfg `loom`, cfg 
`shuttle`, feature `std` (exports all of `std`), feature `alloc` (exports `core` and `alloc`), and
no feature (exports `core`).

[Loom]: https://github.com/tokio-rs/loom
[Shuttle]: https://github.com/awslabs/shuttle

## Installation

Add `gronly-atomics` to your Cargo.toml. If your package is no-std, it is recommended to add
features "std" and "macros" under dev-dependencies. `gronly-atomics` uses the custom cfgs `loom`
and `shuttle` to a select modeling backend, which may cause [unexpected cfg warnings][cfg]. To fix
this, add them to your `check-cfg`.

[cfg]: https://blog.rust-lang.org/2024/05/06/check-cfg.html#expecting-custom-cfgs

```ignore
Cargo.toml

[dependencies]
gronly-atomics = { version = "0.1", features = ["std", "macros"] }

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = ['cfg(loom)', 'cfg(shuttle)'] }
```

## API Differences

 * Modeled atomics do **not** have the same memory layout as thier non-atomic counterparts
 * To fill other api differences, the `AtomicCompat` trait offers the following methods
   - `load_mut`
   - `store_mut`
   - `swap_mut`
   - `debug_load` (only with modeled atomics)

## Usage

The full code for this example can be found [here]. Just import the atomics you want to model from
`gronly-atomics` and write your tests using `#[modeled_test]`, or `#[unmodeled_test]` for the test
to only run in unmodeled contexts. The run your tests as normal to test unmodeled, or set cfg `loom`
or `shuttle` to run them modeled. Without `gronly_atomics` these kind of tests quickly become a
mess of code duplication and cfg gates.

[here]: https://github.com/jahartsell/gronly/tree/main/example-gronly-atomics

```rust,ignore
use gronly_atomics::sync::Arc;
use gronly_atomics::sync::atomic::{AtomicUSize, Ordering};
use gronly_atomics::thread;

#[modeled_test]
fn test_increment() {
    let num = Arc::new(AtomicUsize::new(0));

    let threads: Vec<_> = (0..2)
        .map(|_| {
            let num = Arc::clone(num);
            thread::spawn(move || num.fetch_add(1, Ordering::Relaxed))
        })
        .collect();

    for t in threads {
        t.join().unwrap();
    }

    assert_eq!(num.load(Ordering::Relaxed), 2);
}
```

```bash
# Unmodeled tests are run as usual
cargo test
cargo +nightly miri test

# Tests run modeled with loom
RUSTFLAGS="--cfg loom" cargo test

# Tests run modeled with shuttle
RUSTFLAGS="--cfg shuttle" cargo test
```

## Advanced usage

#### Using other test frameworks

By default the `#[modeled_test]` and `#[unmodeled_test]` macros add `#[test]` to its generated test
cases. To omit this, pass in the `notest` option.

```rust,ignore
#[modeled_test(notest)]
fn test_thing() {
    ...
}
```

#### Using Shuttle schedulers and replays

Shuttle has [many schedulers][sched] available to choose from, by default the random scheduler
with 100 iteratons is used. The other available schedulers are

[sched]: https://docs.rs/shuttle/latest/shuttle/index.html#choosing-a-scheduler-and-running-a-test

```rust,ignore
// Random scheduler, iters defaults to 100
#[modeled_test(shuttle(scheduler = "random", iters = 10))]

// Pct scheduler, iters defaults to 100, depth defaults to 100
#[modeled_test(shuttle(scheduler = "pct", iters = 10, depth = 100))]

// Dfs scheduler, iters defaults to 100
#[modeled_test(shuttle(scheduler = "dfs", iters = 10))]
```

When a shuttle test fails, the error message will print a failing schedule and failing seed. In
order to replay the test, you can set the environment variable `SHUTTLE_RANDOM_SEED` and rerun the
test.

```bash
SHUTTLE_RANDOM_SEED=schedule_seed RUSTFLAGS="--cfg shuttle" cargo test --test test_name
```

Alternatively, you can use the failing scheduler by changing the test attribute to

```rust,ignore
#[modeled_test(shuttle(scheduler(replay = "schedule_hash")))]
```