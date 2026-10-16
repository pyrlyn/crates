#[derive(injecta::Injectable)]
struct Handler;

injecta::container! {
    struct App {
        transient Handler,
    }
}

fn main() {
    let _ = App::new().with(Handler);
}
