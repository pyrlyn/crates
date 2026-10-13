struct Pool;

#[injecta::injectable]
impl Pool {
    #[inject]
    async fn connect() -> Self {
        Self
    }
}

fn main() {}
