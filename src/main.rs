mod http;

use grid_timeline_review::{db, fixture};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut listen = "127.0.0.1:5342".to_string();
    let mut database = "timeline_review.sqlite3".to_string();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--listen" => {
                index += 1;
                listen = args.get(index).cloned().unwrap_or(listen);
            }
            "--db" => {
                index += 1;
                database = args.get(index).cloned().unwrap_or(database);
            }
            "--help" | "-h" => {
                println!(
                    "Usage: grid_timeline_review --listen 127.0.0.1:5342 --db timeline_review.sqlite3"
                );
                return;
            }
            other => eprintln!("ignoring unknown argument: {other}"),
        }
        index += 1;
    }

    let repository = match db::Repository::new(&database) {
        Ok(repository) => repository,
        Err(error) => {
            eprintln!("database initialization failed: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = fixture::seed(&repository) {
        eprintln!("fixture setup failed: {error}");
        std::process::exit(1);
    }
    if let Err(error) = http::serve(repository, &listen) {
        eprintln!("server failed: {error}");
        std::process::exit(1);
    }
}
