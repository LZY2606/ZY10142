use pair_wise_gsb::{db, demo, server, service::App};
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let mut listen = "127.0.0.1:5342".to_string();
    let mut database = PathBuf::from("pair-wise-gsb.sqlite");
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--listen" => {
                index += 1;
                listen = args
                    .get(index)
                    .ok_or("--listen requires an address")?
                    .clone();
            }
            "--db" => {
                index += 1;
                database = PathBuf::from(args.get(index).ok_or("--db requires a path")?);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: pair-wise-gsb [--listen 127.0.0.1:5342] [--db pair-wise-gsb.sqlite]"
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        index += 1;
    }
    let connection = db::open(&database)?;
    let mut app = App::new(connection);
    demo::seed_if_empty(&mut app)?;
    println!("offline disturbance review tool listening on http://{listen}");
    server::serve(app, &listen)
}
