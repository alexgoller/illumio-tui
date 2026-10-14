//! The `:` command language. Parsing only; `update` executes.
use crate::tui::model::Screen;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Screen(Screen),
    /// `:iplists` / `:ip`: the Objects screen on its IP lists tab.
    IpLists,
    Filter(String),
    Window(String),
    /// `:unicast`: the traffic filter keeps unicast flows only; again: every
    /// transmission kind.
    Unicast,
    /// `:focus <app>` narrows every traffic view to one app; `:focus` alone
    /// opens the picker.
    Focus(Option<String>),
    /// `:app <app>` opens the app overlay; `:app` alone opens the picker.
    App(Option<String>),
    Profile(String),
    /// `:writes on` / `:writes off`; bare `:writes` shows the state.
    Writes(Option<bool>),
    /// `:export <path>` writes the table on screen as CSV; `:export!`
    /// overwrites an existing file.
    Export {
        path: String,
        force: bool,
    },
    Draft,
    Active,
    Quit,
    Help,
}

pub fn parse(input: &str) -> Result<Command, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("empty command".into());
    }
    let (name, arg) = match s.split_once(char::is_whitespace) {
        Some((n, a)) => (n, a.trim()),
        None => (s, ""),
    };
    let no_arg = |cmd: Command| {
        if arg.is_empty() {
            Ok(cmd)
        } else {
            Err(format!(":{name} takes no argument"))
        }
    };
    let with_arg = |f: fn(String) -> Command, usage: &str| {
        if arg.is_empty() {
            Err(format!("usage: :{usage}"))
        } else {
            Ok(f(arg.to_string()))
        }
    };
    match name {
        "traffic" | "t" => no_arg(Command::Screen(Screen::Traffic)),
        "policy" | "p" => no_arg(Command::Screen(Screen::Policy)),
        "objects" | "o" | "labels" => no_arg(Command::Screen(Screen::Objects)),
        "workloads" | "w" => no_arg(Command::Screen(Screen::Workloads)),
        "events" | "e" => no_arg(Command::Screen(Screen::Events)),
        "iplists" | "ip" => no_arg(Command::IpLists),
        "filter" | "f" => with_arg(
            Command::Filter,
            "filter <src expr>  (e.g. app=web,env=prod)",
        ),
        "focus" => Ok(Command::Focus((!arg.is_empty()).then(|| arg.to_string()))),
        "app" => Ok(Command::App((!arg.is_empty()).then(|| arg.to_string()))),
        "window" => with_arg(Command::Window, "window <duration>  (e.g. 1h, 24h, 7d)"),
        "profile" => with_arg(Command::Profile, "profile <name>"),
        "writes" => match arg {
            "" => Ok(Command::Writes(None)),
            "on" => Ok(Command::Writes(Some(true))),
            "off" => Ok(Command::Writes(Some(false))),
            _ => Err("usage: :writes [on|off]".into()),
        },
        "export" | "export!" => {
            if arg.is_empty() {
                Err(format!("usage: :{name} <path>"))
            } else {
                Ok(Command::Export {
                    path: arg.to_string(),
                    force: name == "export!",
                })
            }
        }
        "unicast" => no_arg(Command::Unicast),
        "draft" => no_arg(Command::Draft),
        "active" => no_arg(Command::Active),
        "q" | "quit" => no_arg(Command::Quit),
        "help" | "h" | "?" => no_arg(Command::Help),
        other => Err(format!("unknown command :{other}; try :help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::model::Screen;

    #[test]
    fn parses_every_documented_command() {
        assert_eq!(parse("app").unwrap(), Command::App(None));
        assert_eq!(
            parse("app shop/prod").unwrap(),
            Command::App(Some("shop/prod".into()))
        );
        assert_eq!(parse("traffic").unwrap(), Command::Screen(Screen::Traffic));
        assert_eq!(parse("policy").unwrap(), Command::Screen(Screen::Policy));
        assert_eq!(parse("objects").unwrap(), Command::Screen(Screen::Objects));
        assert_eq!(
            parse("workloads").unwrap(),
            Command::Screen(Screen::Workloads)
        );
        assert_eq!(parse("events").unwrap(), Command::Screen(Screen::Events));
        assert_eq!(parse("e").unwrap(), Command::Screen(Screen::Events));
        assert_eq!(parse("iplists").unwrap(), Command::IpLists);
        assert_eq!(parse("ip").unwrap(), Command::IpLists);
        assert_eq!(
            parse("filter app=web,env=prod").unwrap(),
            Command::Filter("app=web,env=prod".into())
        );
        assert_eq!(parse("window 7d").unwrap(), Command::Window("7d".into()));
        assert_eq!(
            parse("profile prod").unwrap(),
            Command::Profile("prod".into())
        );
        assert_eq!(parse("writes").unwrap(), Command::Writes(None));
        assert_eq!(parse("writes on").unwrap(), Command::Writes(Some(true)));
        assert_eq!(parse("writes off").unwrap(), Command::Writes(Some(false)));
        assert_eq!(
            parse("export ~/flows.csv").unwrap(),
            Command::Export {
                path: "~/flows.csv".into(),
                force: false
            }
        );
        assert_eq!(
            parse("export! out dir/a.csv").unwrap(),
            Command::Export {
                path: "out dir/a.csv".into(),
                force: true
            }
        );
        assert_eq!(parse("draft").unwrap(), Command::Draft);
        assert_eq!(parse("unicast").unwrap(), Command::Unicast);
        assert!(parse("unicast now").is_err());
        assert_eq!(parse("active").unwrap(), Command::Active);
        assert_eq!(parse("q").unwrap(), Command::Quit);
        assert_eq!(parse("quit").unwrap(), Command::Quit);
        assert_eq!(parse("help").unwrap(), Command::Help);
        assert_eq!(
            parse("  traffic  ").unwrap(),
            Command::Screen(Screen::Traffic)
        );
        assert_eq!(parse("focus").unwrap(), Command::Focus(None));
        assert_eq!(
            parse("focus shop/prod").unwrap(),
            Command::Focus(Some("shop/prod".into()))
        );
    }

    #[test]
    fn rejects_unknown_and_malformed() {
        assert!(parse("").unwrap_err().contains("empty"));
        assert!(parse("bogus").unwrap_err().contains("unknown command"));
        assert!(parse("filter").unwrap_err().contains("usage"));
        assert!(parse("window").unwrap_err().contains("usage"));
        assert!(parse("profile").unwrap_err().contains("usage"));
        assert_eq!(parse("export").unwrap_err(), "usage: :export <path>");
        assert_eq!(parse("export!").unwrap_err(), "usage: :export! <path>");
        assert_eq!(parse("writes yes").unwrap_err(), "usage: :writes [on|off]");
        assert_eq!(
            parse("writes on now").unwrap_err(),
            "usage: :writes [on|off]"
        );
        assert!(
            parse("traffic extra")
                .unwrap_err()
                .contains("takes no argument")
        );
    }
}
