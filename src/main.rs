mod cli;

use cli::CliArgs;

fn main() {
    match CliArgs::parse() {
        Ok(args) => {
            println!("{:?}", args);
        }
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    }
}
