mod arbitrary;
mod fuzzer;
mod stats;

use std::path::PathBuf;
use std::process::ExitCode;
use std::str::FromStr;
use std::{fs, time::Instant};

use anyhow::{Context, Result};
use argh::FromArgs;
use fuzzer::Fuzzer;
use openapi_utils::SpecExt;
use openapiv3::OpenAPI;
use url::{ParseError, Url};

use crate::fuzzer::FuzzResult;

#[derive(FromArgs, PartialEq, Debug)]
/// openapi-fuzzer - black-box fuzzer that fuzzes APIs based on OpenAPI Specification
struct Cli {
    #[argh(subcommand)]
    subcommands: Subcommands,
}

#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand)]
enum Subcommands {
    Run(RunArgs),
    Resend(ResendArgs),
}

#[derive(FromArgs, Debug, PartialEq)]
/// run openapi-fuzzer
#[argh(subcommand, name = "run")]
struct RunArgs {
    /// path to OpenAPI specification file
    #[argh(option, short = 's')]
    spec: PathBuf,

    /// url of api to fuzz
    #[argh(option, short = 'u')]
    url: UrlWithTrailingSlash,

    /// status codes that will not be considered as finding
    #[argh(option, short = 'i')]
    ignore_status_code: Vec<u16>,

    /// additional header to send
    #[argh(option, short = 'H')]
    header: Vec<Header>,

    /// maximum number of test cases that will run for each combination of endpoint
    /// and method (default: 256)
    #[argh(option, default = "256")]
    max_test_case_count: u32,

    /// directory for results with minimal generated payload used for resending
    /// requests (default: results).
    #[argh(option, short = 'o', default = "String::from(\"results\").into()")]
    results_dir: PathBuf,

    /// directory for request times statistics. if no value is supplied, statistics
    /// will not be saved
    #[argh(option)]
    stats_dir: Option<PathBuf>,
}

#[derive(FromArgs, Debug, PartialEq)]
/// resend payload genereted by fuzzer
#[argh(subcommand, name = "resend")]
struct ResendArgs {
    /// path to result file generated by fuzzer
    #[argh(positional)]
    file: PathBuf,

    /// extra header
    #[argh(option, short = 'H')]
    header: Vec<Header>,

    /// url of api
    #[argh(option, short = 'u')]
    url: UrlWithTrailingSlash,
}

#[derive(Debug, PartialEq)]
struct Header(String, String);

impl FromStr for Header {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = s.splitn(2, ':').collect();
        if parts.len() != 2 {
            return Err("invalid header format".to_string());
        }
        Ok(Header(
            parts[0].to_string().to_lowercase(),
            parts[1].to_string(),
        ))
    }
}

impl From<Header> for (String, String) {
    fn from(val: Header) -> Self {
        (val.0, val.1)
    }
}

#[derive(Debug, PartialEq)]
struct UrlWithTrailingSlash(Url);

impl FromStr for UrlWithTrailingSlash {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.ends_with('/') {
            Ok(UrlWithTrailingSlash(Url::from_str(s)?))
        } else {
            Ok(UrlWithTrailingSlash(Url::from_str(&(s.to_owned() + "/"))?))
        }
    }
}

impl From<UrlWithTrailingSlash> for Url {
    fn from(val: UrlWithTrailingSlash) -> Self {
        val.0
    }
}

fn main() -> Result<ExitCode> {
    let args: Cli = argh::from_env();

    let exit_code = match args.subcommands {
        Subcommands::Run(args) => {
            let specfile = std::fs::read_to_string(&args.spec)
                .context(format!("Unable to read {:?}", &args.spec))?;
            let openapi_schema: OpenAPI =
                serde_yaml::from_str(&specfile).context("Failed to parse schema")?;
            let openapi_schema = openapi_schema.deref_all();

            let now = Instant::now();
            let exit_code = Fuzzer::new(
                openapi_schema,
                args.url.into(),
                args.ignore_status_code,
                args.header.into_iter().map(Into::into).collect(),
                args.max_test_case_count,
                args.results_dir,
                args.stats_dir,
            )
            .run()?;
            println!("Elapsed time: {}s", now.elapsed().as_secs());
            exit_code
        }
        Subcommands::Resend(args) => {
            let json = fs::read_to_string(&args.file)
                .context(format!("Unable to read {:?}", &args.file))?;
            let result: FuzzResult = serde_json::from_str(&json)?;
            let response = Fuzzer::send_request(
                &args.url.into(),
                result.path.to_owned(),
                result.method,
                &result.payload,
                &args.header.into_iter().map(Into::into).collect(),
            )?;
            eprintln!("{} ({})", response.status(), response.status_text());
            println!("{}", response.into_string()?);
            ExitCode::SUCCESS
        }
    };

    Ok(exit_code)
}
