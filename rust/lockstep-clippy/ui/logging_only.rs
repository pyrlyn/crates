// std's print macros are matched by diagnostic item, not by name.

macro_rules! say {
    ($msg:expr) => {
        println!("{}", $msg)
    };
}

macro_rules! println_lookalike {
    ($($arg:tt)*) => {
        format!($($arg)*)
    };
}

fn main() {
    print!("a");
    println!("b");
    eprint!("c");
    eprintln!("d");
    let x = dbg!(1 + 1);

    // A local macro wrapping `println!` is reported inside its definition.
    say!(x);

    // A macro named like a print macro but defined here is not one.
    let _ = println_lookalike!("{}", x);

    // `format!` and `write!` are not logging.
    let _ = format!("{x}");

    quiet();
    allowed_statement();
}

#[allow(lockstep_logging_only)]
fn quiet() {
    println!("allowed for the whole function");
}

fn allowed_statement() {
    #[allow(lockstep_logging_only)]
    let _ = dbg!(2);
    eprintln!("still reported");
}
