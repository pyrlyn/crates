use injecta::Injectable;

#[derive(Injectable)]
struct A {
    b: Box<B>,
}

#[derive(Injectable)]
struct B {
    a: Box<A>,
}

injecta::container! {
    struct App {
        transient Box<A>,
        transient Box<B>,
    }
}

fn main() {}
