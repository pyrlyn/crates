use injecta::Resolve;

#[derive(injecta::Injectable)]
struct Handler;

injecta::container! {
    struct App {
        transient Handler,
    }
}

fn main() {
    let app = App::new();
    let _ = app.get::<Handler>();
}
