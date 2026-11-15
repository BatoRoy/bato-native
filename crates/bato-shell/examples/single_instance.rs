//! A window that is the single instance of its app id: run it, then run it
//! again with arguments. The second launch prints `forwarded` and exits;
//! the first shows (and prints) what it got and raises itself with the
//! second launch's activation token (a token of its own without one).
//!
//! ```sh
//! cargo run -p bato-shell --example single_instance &
//! XDG_ACTIVATION_TOKEN=… cargo run -p bato-shell --example single_instance -- --pick=/run/x.sock ~/Documents
//! ```

use bato_shell::instance::{self, Instance, Primary, Request};
use bato_shell::{AppIdentity, iced_ext};
use iced::widget::{column, text};
use iced::{Element, Subscription, Task, window};

const APP: AppIdentity = AppIdentity::new("Instance demo", "bato-instance-demo", "0.0.0");

struct Demo {
    primary: Primary,
    lines: Vec<String>,
}

#[derive(Debug, Clone)]
enum Msg {
    Forwarded(Request),
    Activated(bool),
    Minted(std::path::PathBuf, Option<String>),
}

impl Demo {
    fn update(&mut self, msg: Msg) -> Task<Msg> {
        match msg {
            Msg::Forwarded(r) => {
                let line = format!(
                    "forwarded: argv={:?} pick={:?} paths={:?} token={:?}",
                    r.argv,
                    r.flag("pick"),
                    r.paths(),
                    r.activation_token
                );
                println!("{line}");
                self.lines.push(line);
                // `--mint=FILE`: act as a launcher, write a token of ours.
                if let Some(file) = r.flag("mint").map(std::path::PathBuf::from) {
                    return window::latest()
                        .and_then(iced_ext::activation_token)
                        .map(move |t| Msg::Minted(file.clone(), t));
                }
                let token = r.activation_token.clone();
                window::latest()
                    .and_then(move |id| iced_ext::activate(id, token.clone()))
                    .map(Msg::Activated)
            }
            Msg::Minted(file, token) => {
                println!("minted token: {token:?}");
                let _ = std::fs::write(file, token.unwrap_or_default());
                Task::none()
            }
            Msg::Activated(sent) => {
                let line = format!("activation request sent: {sent}");
                println!("{line}");
                self.lines.push(line);
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Msg> {
        let mut col =
            column![text(format!("primary on {}", self.primary.socket().display())).size(18)]
                .spacing(8)
                .padding(16);
        for l in &self.lines {
            col = col.push(text(l.clone()).size(14));
        }
        col.into()
    }

    fn subscription(&self) -> Subscription<Msg> {
        iced_ext::instance_requests(&self.primary).map(Msg::Forwarded)
    }
}

fn main() -> iced::Result {
    let primary = match instance::acquire(&APP) {
        Ok(Instance::Primary(p)) => p,
        Ok(Instance::Forwarded) => {
            println!("forwarded");
            return Ok(());
        }
        Err(e) => {
            eprintln!("single instance: {e}");
            std::process::exit(1);
        }
    };
    println!("primary on {}", primary.socket().display());
    iced::application(
        move || Demo {
            primary: primary.clone(),
            lines: Vec::new(),
        },
        Demo::update,
        Demo::view,
    )
    .title(|_: &Demo| String::from(APP.product_name))
    .window(iced_ext::window_settings(&APP))
    .subscription(Demo::subscription)
    .run()
}
