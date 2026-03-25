// SPDX-FileCopyrightText: 2026 Antoni Szymański
// SPDX-License-Identifier: MPL-2.0

use clap::{Parser, Subcommand};
use gitcredential::GitCredential;
use regex::Regex;
use serde::Deserialize;
use serde_with::{BorrowCow, DeserializeAs};
use snafu::{OptionExt, ResultExt, Snafu};
use std::{
    borrow::Cow,
    env, fs, io,
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Return a matching credential, if any exists.
    Get,
    /// Store the credential.
    Store,
    /// Remove matching credentials, if any, from the storage.
    Erase,
}

#[derive(Debug, Snafu)]
#[snafu(context(suffix(Ctx)))]
enum Error {
    #[snafu(display("Failed to parse credential from stdin"))]
    ParseCredential { source: gitcredential::FromReaderError },
    #[snafu(display("Failed to write credential to stdout"))]
    WriteCredential { source: io::Error },
    #[snafu(display("Failed to locate the .git-credentials.json file"))]
    LocateCredentials,
    #[snafu(display("Failed to read credentials from {}", path.display()))]
    ReadCredentials { source: io::Error, path: PathBuf },
    #[snafu(display("Failed to parse from {}", path.display()))]
    ParseCredentials { source: serde_json::Error, path: PathBuf },
}

#[snafu::report]
fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Get => command_get(),
        Commands::Store | Commands::Erase => Ok(()),
    }
}

fn command_get() -> Result<(), Error> {
    let gc = GitCredential::from_reader(io::stdin()).context(ParseCredentialCtx)?;
    let path = &locate_credentials()?;
    let content = match fs::read_to_string(path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        e => e.context(ReadCredentialsCtx { path })?,
    };
    parse_credentials(&content, path)?
        .into_iter()
        .find(|entry| is_match(&gc, entry))
        .map(|entry| GitCredential {
            protocol: gc.protocol.clone(),
            host: gc.host.clone(),
            path: gc.path.clone(),
            username: Some(entry.credentials.username.to_owned()),
            password: Some(entry.credentials.password.to_owned()),
        })
        .map_or_else(|| Ok(()), |gc| gc.to_writer(io::stdout()).context(WriteCredentialCtx))
}

fn locate_credentials() -> Result<PathBuf, Error> {
    match env::var_os("GIT_CREDENTIALS").filter(|s| !s.is_empty()) {
        Some(path) => Ok(path.into()),
        None => env::home_dir()
            .map(|home| home.join(".git-credentials.json"))
            .context(LocateCredentialsCtx),
    }
}

fn parse_credentials<'a>(input: &'a str, path: &Path) -> Result<Vec<Entry<'a>>, Error> {
    serde_json::from_str(input).context(ParseCredentialsCtx { path })
}

fn is_match(gc: &GitCredential, entry: &Entry) -> bool {
    macro_rules! match_fields {
        ($($field:ident),*) => {$(
            if let (Some(value), Some(pattern)) = (gc.$field.as_deref(), &entry.pattern.$field)
                && !pattern.is_match(value)
            {
                return false;
            }
        )*}
    }
    match_fields!(protocol, host, path, username);
    true
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry<'a> {
    pattern: Pattern,
    #[serde(borrow)]
    credentials: Credentials<'a>,
}

#[serde_with::apply(
    Option<Regex> => #[serde_as(as = "Option<RegexSerde>")] #[serde(skip_serializing_if = "Option::is_none")]
)]
#[serde_with::serde_as]
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pattern {
    protocol: Option<Regex>,
    host: Option<Regex>,
    path: Option<Regex>,
    username: Option<Regex>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Credentials<'a> {
    username: &'a str,
    password: &'a str,
}

struct RegexSerde;

impl<'de> DeserializeAs<'de, Regex> for RegexSerde {
    fn deserialize_as<D>(deserializer: D) -> Result<Regex, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let pattern: Cow<str> = BorrowCow::deserialize_as(deserializer)?;
        let anchored_pattern = format!("^(?:{pattern})$");
        Regex::new(&anchored_pattern).map_err(serde::de::Error::custom)
    }
}
