// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2021 The Gleam contributors

use std::io::{Cursor, Read};

use camino::Utf8Path;
use clap::ValueEnum;
use debug_ignore::DebugIgnore;
use ecow::EcoString;
use erlang_consult::Term;
use flate2::read::GzDecoder;
use futures::future;
use hexpm::{ApiError, WriteActionCredentials, version::Version};
use itertools::Itertools;
use strum::{EnumString, VariantNames};
use tar::{Archive, Entry};

use crate::{
    Error, Result, config,
    io::{FileSystemReader, FileSystemWriter, HttpClient, TarUnpacker, WrappedReader},
    manifest::{Base16Checksum, ManifestPackage, ManifestPackageSource},
    paths::{self, ProjectPaths},
};

pub const HEXPM_PUBLIC_KEY: &[u8] = b"-----BEGIN PUBLIC KEY-----
MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEApqREcFDt5vV21JVe2QNB
Edvzk6w36aNFhVGWN5toNJRjRJ6m4hIuG4KaXtDWVLjnvct6MYMfqhC79HAGwyF+
IqR6Q6a5bbFSsImgBJwz1oadoVKD6ZNetAuCIK84cjMrEFRkELtEIPNHblCzUkkM
3rS9+DPlnfG8hBvGi6tvQIuZmXGCxF/73hU0/MyGhbmEjIKRtG6b0sJYKelRLTPW
XgK7s5pESgiwf2YC/2MGDXjAJfpfCd0RpLdvd4eRiXtVlE9qO9bND94E7PgQ/xqZ
J1i2xWFndWa6nfFnRxZmCStCOZWYYPlaxr+FZceFbpMwzTNs4g3d4tLNUcbKAIH4
0wIDAQAB
-----END PUBLIC KEY-----
";

fn key_name(hostname: &str) -> String {
    format!("gleam-{hostname}")
}

pub async fn publish_package<Http: HttpClient>(
    release_tarball: Vec<u8>,
    version: String,
    name: &str,
    api_key: &WriteActionCredentials,
    config: &hexpm::Config,
    replace: bool,
    http: &Http,
) -> Result<()> {
    tracing::info!("Publishing package, replace: {}", replace);
    let request = hexpm::api_publish_package_request(release_tarball, api_key, config, replace);
    let response = http.send(request).await?;
    hexpm::api_publish_package_response(response).map_err(|error| match error {
        ApiError::NotReplacing => Error::HexPublishReplaceRequired { version },
        ApiError::Forbidden => Error::HexPublishAccessDenied {
            name: name.into(),
            version,
        },
        ApiError::Json(_)
        | ApiError::Io(_)
        | ApiError::RateLimited
        | ApiError::InvalidCredentials
        | ApiError::UnexpectedResponse(..)
        | ApiError::InvalidPackageNameFormat(_)
        | ApiError::IncorrectPayloadSignature
        | ApiError::InvalidProtobuf(_)
        | ApiError::InvalidVersionFormat(_)
        | ApiError::NotFound
        | ApiError::InvalidVersionRequirementFormat(_)
        | ApiError::IncorrectChecksum
        | ApiError::OAuthTimeout
        | ApiError::OAuthAccessDenied
        | ApiError::ExpiredToken
        | ApiError::OAuthRefreshTokenRejected
        | ApiError::IncorrectOneTimePassword
        | ApiError::LateDeletion
        | ApiError::LateModification => Error::hex(error),
    })
}

pub async fn add_owner<Http: HttpClient>(
    api_key: &WriteActionCredentials,
    package_name: String,
    new_owner_username_or_email: String,
    level: hexpm::OwnerLevel,
    config: &hexpm::Config,
    http: &Http,
) -> Result<()> {
    tracing::info!(
        "Adding {} as owner of `{}`",
        new_owner_username_or_email,
        package_name
    );
    let request = hexpm::api_add_owner_request(
        &package_name,
        &new_owner_username_or_email,
        level,
        api_key,
        config,
    );
    let response = http.send(request).await?;
    hexpm::api_add_owner_response(response).map_err(Error::hex)
}

pub async fn transfer_owner<Http: HttpClient>(
    api_key: &WriteActionCredentials,
    package_name: String,
    new_owner_username_or_email: String,
    config: &hexpm::Config,
    http: &Http,
) -> Result<()> {
    tracing::info!(
        "Transferring ownership of `{}` to {}",
        package_name,
        new_owner_username_or_email
    );
    let request = hexpm::api_transfer_owner_request(
        &package_name,
        &new_owner_username_or_email,
        api_key,
        config,
    );
    let response = http.send(request).await?;
    hexpm::api_transfer_owner_response(response).map_err(Error::hex)
}

#[derive(Debug, EnumString, VariantNames, ValueEnum, Clone, Copy, PartialEq, Eq)]
#[strum(serialize_all = "lowercase")]
pub enum RetirementReason {
    Other,
    Invalid,
    Security,
    Deprecated,
    Renamed,
}

impl RetirementReason {
    pub fn to_library_enum(&self) -> hexpm::RetirementReason {
        match self {
            RetirementReason::Other => hexpm::RetirementReason::Other,
            RetirementReason::Invalid => hexpm::RetirementReason::Invalid,
            RetirementReason::Security => hexpm::RetirementReason::Security,
            RetirementReason::Deprecated => hexpm::RetirementReason::Deprecated,
            RetirementReason::Renamed => hexpm::RetirementReason::Renamed,
        }
    }
}

pub async fn retire_release<Http: HttpClient>(
    package: &str,
    version: &str,
    reason: RetirementReason,
    message: Option<&str>,
    api_key: &WriteActionCredentials,
    config: &hexpm::Config,
    http: &Http,
) -> Result<()> {
    tracing::info!(package=%package, version=%version, "retiring_hex_release");
    let request = hexpm::api_retire_release_request(
        package,
        version,
        reason.to_library_enum(),
        message,
        api_key,
        config,
    );
    let response = http.send(request).await?;
    hexpm::api_retire_release_response(response).map_err(Error::hex)
}

pub async fn unretire_release<Http: HttpClient>(
    package: &str,
    version: &str,
    api_key: &WriteActionCredentials,
    config: &hexpm::Config,
    http: &Http,
) -> Result<()> {
    tracing::info!(package=%package, version=%version, "retiring_hex_release");
    let request = hexpm::api_unretire_release_request(package, version, api_key, config);
    let response = http.send(request).await?;
    hexpm::api_unretire_release_response(response).map_err(Error::hex)
}

pub async fn remove_api_key<Http: HttpClient>(
    hostname: &str,
    config: &hexpm::Config,
    auth_key: &WriteActionCredentials,
    http: &Http,
) -> Result<()> {
    tracing::info!("Deleting API key from Hex");
    let request = hexpm::api_remove_api_key_request(&key_name(hostname), auth_key, config);
    let response = http.send(request).await?;
    hexpm::api_remove_api_key_response(response).map_err(Error::hex)
}

#[derive(Debug)]
pub struct Downloader {
    fs_reader: DebugIgnore<Box<dyn FileSystemReader>>,
    fs_writer: DebugIgnore<Box<dyn FileSystemWriter>>,
    http: DebugIgnore<Box<dyn HttpClient>>,
    untar: DebugIgnore<Box<dyn TarUnpacker>>,
    hex_config: hexpm::Config,
    credentials: DebugIgnore<Option<hexpm::Credentials>>,
    paths: ProjectPaths,
}

impl Downloader {
    pub fn new(
        fs_reader: Box<dyn FileSystemReader>,
        fs_writer: Box<dyn FileSystemWriter>,
        http: Box<dyn HttpClient>,
        untar: Box<dyn TarUnpacker>,
        credentials: Option<hexpm::Credentials>,
        paths: ProjectPaths,
    ) -> Self {
        Self {
            fs_reader: DebugIgnore(fs_reader),
            fs_writer: DebugIgnore(fs_writer),
            http: DebugIgnore(http),
            untar: DebugIgnore(untar),
            hex_config: hexpm::Config::new(),
            credentials: DebugIgnore(credentials),
            paths,
        }
    }

    /// Read a package's outer tarball from the global cache, downloading it
    /// first if it is missing or does not match `outer_checksum`, which comes
    /// from the signed registry. Checking the cached copy means a partially
    /// written file gets replaced instead of trusted.
    async fn cached_tarball(
        &self,
        name: &str,
        version: &Version,
        outer_checksum: &Base16Checksum,
    ) -> Result<Vec<u8>> {
        let tarball_path = paths::global_package_cache_package_tarball(outer_checksum);
        if self.fs_reader.is_file(&tarball_path) {
            let tarball = self.fs_reader.read_bytes(&tarball_path)?;
            if hexpm::verify_package_tarball(&tarball, &outer_checksum.0).is_ok() {
                tracing::info!(package = name, version = %version, "package_in_cache");
                return Ok(tarball);
            }
            tracing::info!(package = name, version = %version, "cached_package_checksum_mismatch");
        }
        tracing::info!(
            package = name,
            version = %version,
            "downloading_package_to_cache"
        );

        let request = hexpm::repository_get_package_tarball_request(
            name,
            &version.to_string(),
            self.credentials.as_ref(),
            &self.hex_config,
        );
        let response = self.http.send(request).await?;

        let tarball = hexpm::repository_get_package_tarball_response(response, &outer_checksum.0)
            .map_err(|error| Error::DownloadPackageError {
            package_name: name.to_string(),
            package_version: version.to_string(),
            error: error.to_string(),
        })?;
        self.fs_writer.write_bytes(&tarball_path, &tarball)?;
        Ok(tarball)
    }

    /// Call `f` with the first entry called `entry_name` in an outer tarball.
    fn with_tarball_entry<T>(
        &self,
        outer_checksum: &Base16Checksum,
        tarball: &[u8],
        entry_name: &str,
        f: impl FnOnce(Entry<'_, WrappedReader>) -> Result<T>,
    ) -> Result<T> {
        let tarball_path = paths::global_package_cache_package_tarball(outer_checksum);
        let mut archive = Archive::new(WrappedReader::new(
            &tarball_path,
            Box::new(Cursor::new(tarball.to_vec())),
        ));

        for entry in self.untar.entries(&mut archive)? {
            let entry = entry.map_err(Error::expand_tar)?;
            let path = entry.header().path().map_err(Error::expand_tar)?;
            if path.as_ref() == Utf8Path::new(entry_name) {
                return f(entry);
            }
        }
        Err(Error::ExpandTar {
            error: format!("Unable to locate Hex package {entry_name}"),
        })
    }

    /// Set the package's OTP application name and build tools from the
    /// `metadata.config` in its tarball, downloading it to the cache first if
    /// needed.
    pub async fn read_release_meta(&self, package: &mut ManifestPackage) -> Result<()> {
        let outer_checksum = hex_outer_checksum(package);
        let tarball = self
            .cached_tarball(&package.name, &package.version, outer_checksum)
            .await?;
        let config =
            self.with_tarball_entry(outer_checksum, &tarball, "metadata.config", |mut file| {
                let mut config = Vec::new();
                let _ = file.read_to_end(&mut config).map_err(Error::expand_tar)?;
                Ok(config)
            })?;
        let meta =
            release_meta_from_config(&config).map_err(|error| Error::DownloadPackageError {
                package_name: package.name.to_string(),
                package_version: package.version.to_string(),
                error,
            })?;

        package.otp_app = Some(meta.app);
        package.build_tools = meta.build_tools;
        Ok(())
    }

    pub async fn ensure_package_in_build_directory(
        &self,
        package: &ManifestPackage,
    ) -> Result<bool> {
        let destination = self.paths.build_packages_package(&package.name);

        // If the directory already exists then there's nothing for us to do
        if self.fs_reader.is_directory(&destination) {
            tracing::info!(
                package = package.name.as_str(),
                "Package already in build directory"
            );
            return Ok(false);
        }

        let outer_checksum = hex_outer_checksum(package);
        let tarball = self
            .cached_tarball(&package.name, &package.version, outer_checksum)
            .await?;

        tracing::info!(package = package.name.as_str(), "writing_package_to_target");
        self.with_tarball_entry(outer_checksum, &tarball, "contents.tar.gz", |file| {
            // Expand the inner source code and write to the file system
            let archive = Archive::new(GzDecoder::new(file));
            let result = self.untar.unpack(&destination, archive);

            // If we failed to expand the tarball remove any source code
            // that was partially written so that we don't mistakenly think
            // the operation succeeded next time we run.
            match result {
                Ok(()) => Ok(true),
                Err(err) => {
                    self.fs_writer.delete_directory(&destination)?;
                    Err(err)
                }
            }
        })
    }

    pub async fn download_hex_packages<'a, Packages: Iterator<Item = &'a ManifestPackage>>(
        &self,
        packages: Packages,
        project_name: &str,
    ) -> Result<()> {
        let futures = packages
            .filter(|package| project_name != package.name)
            .map(|package| self.ensure_package_in_build_directory(package));

        // Run the futures to download the packages concurrently
        let results = future::join_all(futures).await;

        // Count the number of packages downloaded while checking for errors
        for result in results {
            let _ = result?;
        }
        Ok(())
    }
}

fn hex_outer_checksum(package: &ManifestPackage) -> &Base16Checksum {
    match &package.source {
        ManifestPackageSource::Hex { outer_checksum } => outer_checksum,
        ManifestPackageSource::Git { .. } | ManifestPackageSource::Local { .. } => {
            panic!("Attempt to download non-hex package from hex")
        }
    }
}

pub async fn publish_documentation<Http: HttpClient>(
    name: &str,
    version: &Version,
    archive: Vec<u8>,
    api_key: &WriteActionCredentials,
    config: &hexpm::Config,
    http: &Http,
) -> Result<()> {
    tracing::info!("publishing_documentation");
    let request =
        hexpm::api_publish_docs_request(name, &version.to_string(), archive, api_key, config)
            .map_err(Error::hex)?;
    let response = http.send(request).await?;
    hexpm::api_publish_docs_response(response).map_err(Error::hex)
}

#[derive(Debug, PartialEq, Eq)]
struct ReleaseMeta {
    app: EcoString,
    build_tools: Vec<EcoString>,
}

/// Files that Hex uses to guess the build tools of a release whose metadata
/// doesn't list them.
const BUILD_TOOL_FILES: [(&str, &str); 5] = [
    ("mix.exs", "mix"),
    ("rebar.config", "rebar3"),
    ("rebar", "rebar3"),
    ("Makefile", "make"),
    ("Makefile.win", "make"),
];

/// Read the `app` and `build_tools` entries from a Hex tarball's
/// `metadata.config`.
fn release_meta_from_config(config: &[u8]) -> Result<ReleaseMeta, String> {
    let terms = erlang_consult::read_all(config)
        .map_err(|error| format!("Unable to parse Hex package metadata.config: {error}"))?;

    let mut app = None;
    let mut build_tools = None;
    let mut files = None;
    for term in &terms {
        let Term::Tuple(pair) = term else { continue };
        let [key, value] = pair.as_slice() else {
            continue;
        };
        match binary_text(key) {
            Some("app") => app = binary_text(value).map(EcoString::from),
            Some("build_tools") => build_tools = Some(value),
            Some("files") => files = Some(value),
            _ => {}
        }
    }

    let app = app
        .filter(|app| config::is_valid_package_name(app))
        .ok_or("Hex package metadata.config has a missing or invalid app")?;

    // Releases published by early versions of Hex don't list their build
    // tools, so like Hex we guess them from the top-level files.
    let build_tools = match (build_tools, files) {
        (Some(Term::List(tools)), _) => tools
            .iter()
            .map(|tool| binary_text(tool).map(EcoString::from))
            .collect::<Option<Vec<_>>>()
            .ok_or("Hex package metadata.config has invalid build_tools")?
            .into_iter()
            .unique()
            .collect(),
        (_, Some(Term::List(files))) => guess_build_tools(files),
        _ => return Err("Hex package metadata.config has missing build_tools".into()),
    };

    Ok(ReleaseMeta { app, build_tools })
}

/// Hex only reads UTF-8 binaries as text, so a charlist such as `"app"` is a
/// different key from `<<"app">>`.
fn binary_text(term: &Term) -> Option<&str> {
    match term {
        Term::Binary(bytes) => std::str::from_utf8(bytes).ok(),
        Term::Atom(_) | Term::Tuple(_) | Term::List(_) | Term::Map(_) | Term::Integer(_) => None,
    }
}

/// Guess build tools from a release's top-level files the way Hex does,
/// returning them sorted and without duplicates.
fn guess_build_tools(files: &[Term]) -> Vec<EcoString> {
    BUILD_TOOL_FILES
        .iter()
        .filter(|(file, _)| files.iter().any(|name| binary_text(name) == Some(*file)))
        .map(|(_, tool)| *tool)
        .sorted()
        .dedup()
        .map(EcoString::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::memory::InMemoryFileSystem;
    use camino::Utf8PathBuf;
    use std::sync::{Arc, Mutex};

    #[test]
    fn release_meta_from_gleam_config() {
        let config = br#"{<<"name">>, <<"wibble"/utf8>>}.
{<<"app">>, <<"wibble"/utf8>>}.
{<<"build_tools">>, [<<"gleam"/utf8>>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["gleam".into()],
            })
        );
    }

    #[test]
    fn release_meta_from_mix_config() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,[<<"mix">>,<<"rebar3">>,<<"mix">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["mix".into(), "rebar3".into()],
            })
        );
    }

    #[test]
    fn release_meta_guesses_build_tools_from_files() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"files">>,[<<"src/mix.exs">>,<<"rebar.config">>,<<"Makefile">>,<<"mix.exs">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["make".into(), "mix".into(), "rebar3".into()],
            })
        );
    }

    #[test]
    fn release_meta_guesses_build_tools_when_not_a_list() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,<<"mix">>}.
{<<"files">>,[<<"rebar.config">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["rebar3".into()],
            })
        );
    }

    #[test]
    fn release_meta_rejects_invalid_app() {
        let config = br#"{<<"app">>,<<"../wibble">>}.
{<<"build_tools">>,[<<"mix">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Err("Hex package metadata.config has a missing or invalid app".into())
        );
    }

    #[test]
    fn release_meta_rejects_missing_app() {
        let config = br#"{<<"build_tools">>,[<<"mix">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Err("Hex package metadata.config has a missing or invalid app".into())
        );
    }

    #[test]
    fn release_meta_rejects_missing_build_tools_and_files() {
        let config = br#"{<<"app">>,<<"wibble">>}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Err("Hex package metadata.config has missing build_tools".into())
        );
    }

    #[test]
    fn release_meta_rejects_non_text_build_tool() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,[mix]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Err("Hex package metadata.config has invalid build_tools".into())
        );
    }

    #[test]
    fn release_meta_preserves_build_tool_labels() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,[<<"rebar3">>,<<"erlang.mk">>,<<"tool-name">>,<<"tool\"name">>]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec![
                    "rebar3".into(),
                    "erlang.mk".into(),
                    "tool-name".into(),
                    "tool\"name".into(),
                ],
            })
        );
    }

    #[test]
    fn release_meta_ignores_charlist_keys() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,[<<"gleam">>]}.
{"build_tools",[<<"rebar3">>]}.
{"app",<<"wobble">>}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["gleam".into()],
            })
        );
    }

    #[test]
    fn release_meta_ignores_quoted_atoms() {
        let config = br#"{<<"app">>,<<"wibble">>}.
{<<"build_tools">>,[<<"gleam">>]}.
{<<"extra">>,[{<<"flag">>,'true'}]}.
"#;
        assert_eq!(
            release_meta_from_config(config),
            Ok(ReleaseMeta {
                app: "wibble".into(),
                build_tools: vec!["gleam".into()],
            })
        );
    }

    struct Untar;

    impl TarUnpacker for Untar {
        fn io_result_entries<'a>(
            &self,
            archive: &'a mut Archive<WrappedReader>,
        ) -> std::io::Result<tar::Entries<'a, WrappedReader>> {
            archive.entries()
        }

        fn io_result_unpack(
            &self,
            _path: &Utf8Path,
            _archive: Archive<GzDecoder<Entry<'_, WrappedReader>>>,
        ) -> std::io::Result<()> {
            unreachable!("these tests never unpack a package")
        }
    }

    /// Serves one tarball and records the authorization header of each
    /// request for it.
    struct TarballHttpClient {
        tarball: Vec<u8>,
        authorizations: Arc<Mutex<Vec<Option<String>>>>,
    }

    #[async_trait::async_trait]
    impl HttpClient for TarballHttpClient {
        async fn send(
            &self,
            request: http::Request<Vec<u8>>,
        ) -> Result<http::Response<Vec<u8>>, Error> {
            let authorization = request
                .headers()
                .get("authorization")
                .map(|value| value.to_str().unwrap().to_string());
            self.authorizations.lock().unwrap().push(authorization);
            Ok(http::Response::builder()
                .status(200)
                .body(self.tarball.clone())
                .unwrap())
        }
    }

    /// A downloader serving `tarball`, along with the authorization header of
    /// each request it makes.
    fn downloader(
        fs: &InMemoryFileSystem,
        tarball: Vec<u8>,
        credentials: Option<hexpm::Credentials>,
    ) -> (Downloader, Arc<Mutex<Vec<Option<String>>>>) {
        let authorizations = Arc::new(Mutex::new(vec![]));
        let http = TarballHttpClient {
            tarball,
            authorizations: authorizations.clone(),
        };
        let downloader = Downloader::new(
            Box::new(fs.clone()),
            Box::new(fs.clone()),
            Box::new(http),
            Box::new(Untar),
            credentials,
            ProjectPaths::new(Utf8PathBuf::from("/project")),
        );
        (downloader, authorizations)
    }

    fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            builder.append_data(&mut header, path, *contents).unwrap();
        }
        builder.into_inner().unwrap()
    }

    /// The sha256 of a fixture tarball. These are fixed because `tarball`
    /// builds the same bytes for the same entries; if a fixture changes, its
    /// checksum must be recomputed.
    fn checksum(sha256: &str) -> Base16Checksum {
        Base16Checksum(base16::decode(sha256).unwrap())
    }

    fn package(outer_checksum: Base16Checksum) -> ManifestPackage {
        ManifestPackage {
            name: "wibble".into(),
            version: Version::new(1, 0, 0),
            build_tools: vec![],
            otp_app: None,
            requirements: vec![],
            source: ManifestPackageSource::Hex { outer_checksum },
        }
    }

    /// Fill in the package's release metadata, returning how many times the
    /// tarball was downloaded.
    fn read_meta(
        fs: &InMemoryFileSystem,
        tarball: Vec<u8>,
        package: &mut ManifestPackage,
    ) -> Result<usize> {
        let (downloader, authorizations) = downloader(fs, tarball, None);
        futures::executor::block_on(downloader.read_release_meta(package))?;
        Ok(authorizations.lock().unwrap().len())
    }

    const METADATA: &[u8] = br#"{<<"app">>,<<"wibble_app">>}.
{<<"build_tools">>,[<<"rebar3">>]}.
"#;

    const METADATA_TARBALL_SHA256: &str =
        "CA60E38E6E4F85698A3A9B30049CC33EDDA7C18F2412CCA9F5813B9E6D51E66A";

    #[test]
    fn read_release_meta_uses_valid_cached_tarball() {
        let tarball = tarball(&[("metadata.config", METADATA)]);
        let outer_checksum = checksum(METADATA_TARBALL_SHA256);
        let fs = InMemoryFileSystem::new();
        fs.write_bytes(
            &paths::global_package_cache_package_tarball(&outer_checksum),
            &tarball,
        )
        .unwrap();
        let mut package = package(outer_checksum);

        assert_eq!(read_meta(&fs, vec![], &mut package), Ok(0));
        assert_eq!(package.otp_app, Some("wibble_app".into()));
        assert_eq!(package.build_tools, vec![EcoString::from("rebar3")]);
    }

    #[test]
    fn read_release_meta_replaces_truncated_cached_tarball() {
        let tarball = tarball(&[("metadata.config", METADATA)]);
        let outer_checksum = checksum(METADATA_TARBALL_SHA256);
        let cache_path = paths::global_package_cache_package_tarball(&outer_checksum);
        let fs = InMemoryFileSystem::new();
        fs.write_bytes(&cache_path, &tarball[..100]).unwrap();
        let mut package = package(outer_checksum);

        assert_eq!(read_meta(&fs, tarball.clone(), &mut package), Ok(1));
        assert_eq!(package.otp_app, Some("wibble_app".into()));
        assert_eq!(fs.read_bytes(&cache_path), Ok(tarball));
    }

    #[test]
    fn read_release_meta_uses_first_metadata_entry() {
        let decoy = br#"{<<"app">>,<<"wobble_app">>}.
{<<"build_tools">>,[<<"gleam">>]}.
"#;
        let tarball = tarball(&[("metadata.config", METADATA), ("metadata.config", decoy)]);
        let outer_checksum =
            checksum("AA7063E2F2433CD4851286C84CF488F21F49B61D0142D4CBF8F6E97B66C6C6E3");
        let mut package = package(outer_checksum);

        assert_eq!(
            read_meta(&InMemoryFileSystem::new(), tarball, &mut package),
            Ok(1)
        );
        assert_eq!(package.otp_app, Some("wibble_app".into()));
        assert_eq!(package.build_tools, vec![EcoString::from("rebar3")]);
    }

    #[test]
    fn read_release_meta_ignores_unreadable_trailing_entry() {
        let mut tarball = tarball(&[("metadata.config", METADATA)]);
        let end_of_archive = tarball.split_off(tarball.len() - 1024);
        tarball.extend([b'x'; 512]);
        tarball.extend(end_of_archive);
        let outer_checksum =
            checksum("4D6B2E2E718B5E7D27E6FC1DF4AD56219EC897E49EFE4FA0F0E61BCC81866D23");
        let mut package = package(outer_checksum);

        assert_eq!(
            read_meta(&InMemoryFileSystem::new(), tarball, &mut package),
            Ok(1)
        );
        assert_eq!(package.otp_app, Some("wibble_app".into()));
    }

    #[test]
    fn read_release_meta_reports_missing_metadata() {
        let tarball = tarball(&[("VERSION", b"3")]);
        let outer_checksum =
            checksum("CD2FF8CF0F32190AC134C71A7442B4CCF6538E29BC583039F2C685B36095043D");
        let mut package = package(outer_checksum);

        assert_eq!(
            read_meta(&InMemoryFileSystem::new(), tarball, &mut package),
            Err(Error::ExpandTar {
                error: "Unable to locate Hex package metadata.config".into()
            })
        );
    }

    #[test]
    fn read_release_meta_sends_read_api_key() {
        let tarball = tarball(&[("metadata.config", METADATA)]);
        let fs = InMemoryFileSystem::new();
        let credentials = hexpm::Credentials::ApiKey("secret-key".into());
        let (downloader, authorizations) = downloader(&fs, tarball, Some(credentials));
        let mut package = package(checksum(METADATA_TARBALL_SHA256));

        futures::executor::block_on(downloader.read_release_meta(&mut package)).unwrap();

        assert_eq!(
            *authorizations.lock().unwrap(),
            vec![Some("secret-key".to_string())]
        );
    }

    #[test]
    fn read_release_meta_is_anonymous_without_api_key() {
        let tarball = tarball(&[("metadata.config", METADATA)]);
        let fs = InMemoryFileSystem::new();
        let (downloader, authorizations) = downloader(&fs, tarball, None);
        let mut package = package(checksum(METADATA_TARBALL_SHA256));

        futures::executor::block_on(downloader.read_release_meta(&mut package)).unwrap();

        assert_eq!(*authorizations.lock().unwrap(), vec![None]);
    }

    #[test]
    fn ensure_package_in_build_directory_skips_existing_directory() {
        let fs = InMemoryFileSystem::new();
        fs.mkdir(&ProjectPaths::new("/project".into()).build_packages_package("wibble"))
            .unwrap();
        let (downloader, authorizations) = downloader(&fs, vec![], None);
        let package = package(checksum(METADATA_TARBALL_SHA256));

        assert_eq!(
            futures::executor::block_on(downloader.ensure_package_in_build_directory(&package)),
            Ok(false)
        );
        assert_eq!(*authorizations.lock().unwrap(), vec![]);
    }
}
