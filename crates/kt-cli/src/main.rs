//! `kt`: the Kitchen Table command line.
//!
//! A client of the daemon socket, like the shell and like agents. Anything the
//! UI can do is reachable from here, which is the rule that keeps the socket
//! honest (docs/architecture.md section 1).

use std::process::ExitCode;

use kt_types::{paths, App, ServingState};

mod client;
mod qr;
mod table;

use client::{Client, ClientError};

const USAGE: &str = "\
kt - a home for your small software

Usage:
  kt list        Everything in your workspace, and where to reach it
  kt url <app>   The app's URL, with a QR code to point a phone at
  kt status      Whether the daemon is serving, and from which folder
  kt help        This message
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("help");

    let result = match command {
        "list" | "ls" => run(list),
        "url" => match args.get(1) {
            Some(slug) => {
                let slug = slug.clone();
                run(move |c| url(c, &slug))
            }
            None => {
                eprintln!("kt url: which app?\n\n  kt url trip-planner\n");
                return ExitCode::from(2);
            }
        },
        "status" => run(status),
        "help" | "-h" | "--help" => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => {
            eprintln!("kt: unknown command {other:?}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn run(f: impl Fn(&mut Client) -> Result<(), ClientError>) -> Result<(), ClientError> {
    let home = std::env::var("HOME")
        .map_err(|_| ClientError::Io(std::io::Error::other("HOME is not set")))?;
    let mut client = Client::connect(&paths::socket_path(&home))?;
    f(&mut client)
}

fn list(client: &mut Client) -> Result<(), ClientError> {
    let apps: Vec<App> = serde_json::from_value(client.call("app.list", None)?)?;

    if apps.is_empty() {
        println!("No apps yet. Drop a folder into your workspace and it becomes one.");
        return Ok(());
    }

    let rows: Vec<[String; 4]> = apps
        .iter()
        .map(|a| {
            [
                a.slug.clone(),
                a.visibility.label().to_string(),
                format!("v{}", a.version),
                a.url.clone(),
            ]
        })
        .collect();

    table::print(["APP", "VISIBILITY", "VERSION", "URL"], &rows);
    Ok(())
}

/// The URL and a scannable code. The QR is the point: it is how an app gets
/// from a laptop to a phone without anyone typing anything.
fn url(client: &mut Client, slug: &str) -> Result<(), ClientError> {
    let app: App =
        serde_json::from_value(client.call("app.get", Some(serde_json::json!({ "slug": slug })))?)?;

    println!();
    println!("{}", qr::render(&app.url));
    println!("  {}", app.url);
    if app.url != app.fallback_url {
        println!("  {}   (if .local does not resolve)", app.fallback_url);
    }
    if let Some(hostname) = &app.hostname {
        if !hostname.starts_with(&app.slug) {
            println!("  announced as {hostname} - the name you wanted was taken");
        }
    }
    println!();
    Ok(())
}

fn status(client: &mut Client) -> Result<(), ClientError> {
    let status: kt_types::SysStatus = serde_json::from_value(client.call("sys.status", None)?)?;

    let serving = match &status.serving {
        ServingState::Serving => "serving".to_string(),
        ServingState::Paused => "paused".to_string(),
        ServingState::Degraded { message, .. } => format!("degraded - {message}"),
    };

    println!("{:<12} {}", "status", serving);
    println!("{:<12} {}", "workspace", status.workspace);
    println!("{:<12} {}", "apps", status.app_count);
    println!(
        "{:<12} {} (protocol {})",
        "daemon", status.daemon_version, status.protocol_version
    );
    Ok(())
}
