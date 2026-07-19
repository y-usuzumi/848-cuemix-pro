mod avdecc;
mod cli;
mod device;
mod discovery;
mod probe;
mod server;
mod ui;

fn main() {
    if let Err(error) = cli::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
