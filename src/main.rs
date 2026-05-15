// SPDX-FileCopyrightText: 2026 Antoni Szymański
// SPDX-License-Identifier: MPL-2.0

use clap::{Parser, Subcommand};
use gitcredential::GitCredential;
use regex::Regex;
use serde::Deserialize;
use serde_with::{BorrowCow, DeserializeAs};
use snafu::{ResultExt, Snafu};
use std::{
    borrow::Cow,
    env,
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(long)]
    file: Option<PathBuf>,
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
    #[snafu(display("Failed to open the credentials file"))]
    OpenCredentials { source: io::Error },
    #[snafu(display("Failed to read credentials from {}", path.display()))]
    ReadCredentials { source: io::Error, path: PathBuf },
    #[snafu(display("Failed to parse from {}", path.display()))]
    ParseCredentials { source: serde_json::Error, path: PathBuf },
}

#[snafu::report]
fn main() -> Result<(), Error> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Get => command_get(cli.file),
        Commands::Store | Commands::Erase => Ok(()),
    }
}

fn command_get(file: Option<PathBuf>) -> Result<(), Error> {
    let gc = GitCredential::from_reader(io::stdin()).context(ParseCredentialCtx)?;
    let (mut file, path) = match open_credentials(file).context(OpenCredentialsCtx)? {
        Some(v) => v,
        None => return Ok(()),
    };
    let mut content = String::new();
    file.read_to_string(&mut content).context(ReadCredentialsCtx { path: &path })?;
    parse_credentials(&content, &path)?
        .into_iter()
        .find(|entry| is_match(&gc, entry))
        .map(|entry| GitCredential {
            protocol: gc.protocol,
            host: gc.host,
            path: gc.path,
            username: Some(entry.credentials.username.into_owned()),
            password: Some(entry.credentials.password.into_owned()),
        })
        .map_or_else(|| Ok(()), |gc| gc.to_writer(io::stdout()).context(WriteCredentialCtx))
}

fn open_credentials(file: Option<PathBuf>) -> Result<Option<(File, PathBuf)>, io::Error> {
    macro_rules! try_open {
        ($($source:expr),*) => {$(
            if let Some(path) = $source {
                match File::open(&path) {
                    Ok(file) => return Ok(Some((file, path))),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e),
                }
            }
        )*};
    }
    try_open!(
        file,
        env::var_os("GIT_CREDENTIALS").filter(|s| !s.is_empty()).map(PathBuf::from),
        dirs::config_dir().map(|p| p.join("git").join("credentials.json")),
        dirs::home_dir().map(|p| p.join(".git-credentials.json"))
    );
    Ok(None)
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
        )*};
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
    username: Cow<'a, str>,
    password: Cow<'a, str>,
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
