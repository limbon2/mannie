//! The output of `git remote -v`: each remote with the URL that it fetches
//! from and the one that it pushes to, one line for each, such as
//! `origin\thttps://example.com/app.git (fetch)`. URLs lose their
//! credentials as they are read (T-47).

use serde::Serialize;

use crate::redact::redact;

/// A remote and its URLs, without credentials.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Remote {
    pub(crate) name: String,
    pub(crate) fetch: Option<String>,
    pub(crate) push: Option<String>,
}

/// Reads the output of `git remote -v`, keeping the order of the remotes.
pub(crate) fn parse_remotes(output: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in output.lines() {
        let Some((name, rest)) = line.split_once('\t') else {
            continue;
        };
        let Some((url, direction)) = rest.rsplit_once(' ') else {
            continue;
        };
        let url = redact(url);
        let known = remotes.iter().position(|remote| remote.name == name);
        let index = known.unwrap_or_else(|| {
            remotes.push(Remote {
                name: name.to_owned(),
                ..Remote::default()
            });
            remotes.len() - 1
        });
        let remote = &mut remotes[index];
        match direction {
            "(fetch)" => remote.fetch = Some(url),
            "(push)" => remote.push = Some(url),
            _other => {}
        }
    }
    remotes
}

#[cfg(test)]
mod tests {
    use super::{Remote, parse_remotes};

    #[test]
    fn each_remote_keeps_its_urls_without_credentials() {
        let output = "origin\thttps://me:token@example.com/app.git (fetch)\n\
origin\thttps://me:token@example.com/app.git (push)\n\
mirror\tgit@example.com:app.git (fetch)\n\
mirror\tssh://git:secret@example.com/app.git (push)\n";

        let remotes = parse_remotes(output);

        assert_eq!(
            remotes,
            [
                Remote {
                    name: "origin".to_owned(),
                    fetch: Some("https://example.com/app.git".to_owned()),
                    push: Some("https://example.com/app.git".to_owned()),
                },
                Remote {
                    name: "mirror".to_owned(),
                    fetch: Some("git@example.com:app.git".to_owned()),
                    push: Some("ssh://git@example.com/app.git".to_owned()),
                },
            ]
        );
        assert_eq!(parse_remotes(""), Vec::new());
    }
}
