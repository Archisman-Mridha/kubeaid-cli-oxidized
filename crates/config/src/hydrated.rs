use {
  crate::raw,
  gix_url::{Scheme, Url},
  std::{fs,
        io::Write,
        os::unix::net::UnixStream,
        path::PathBuf,
        process::{Command, Stdio}}
};

const DEFAULT_GIT_SSH_USERNAME: &str = "git";
const DEFAULT_KUBEAID_REPOSITORY_URL: &str = "https://github.com/Obmondo/KubeAid";

// o2o's attribute parser can't handle the angle brackets of `Box<dyn ...>`, so the derived
// conversions refer to the error type through this alias.
type BoxedError = Box<dyn std::error::Error>;

// A `Config` is just a validated `raw::Config`, field-for-field, so its `TryFrom` is derived : the
// only field recurses into `Repositories`'s (hand-written) `TryFrom`.
#[derive(o2o::o2o)]
#[try_from_owned(raw::Config, BoxedError)]
pub struct Config {
  #[from(~.ok_or("The repositories section is missing from the config")?.try_into()?)]
  pub repositories: Repositories
}

pub struct Repositories {
  pub ssh_access: Option<SSHAccess>,

  pub kubeaid:        KubeAid,
  pub kubeaid_config: KubeAidConfig
}

impl TryFrom<raw::Repositories> for Repositories {
  type Error = Box<dyn std::error::Error>;

  fn try_from(repositories: raw::Repositories) -> Result<Self, Self::Error> {
    let ssh_access: Option<SSHAccess> =
      repositories.ssh_access.map(|ssh_access| ssh_access.try_into()).transpose()?;

    let (kubeaid_url, kubeaid_version) =
      repositories.kubeaid.map_or((None, None), |kubeaid| (kubeaid.url, kubeaid.version));

    let kubeaid_url: RepositoryURL =
      kubeaid_url.unwrap_or_else(|| DEFAULT_KUBEAID_REPOSITORY_URL.into()).try_into()?;
    let kubeaid = KubeAid::new(kubeaid_url, kubeaid_version, ssh_access.as_ref())?;

    let kubeaid_config_url: RepositoryURL =
      repositories.kubeaid_config
                  .ok_or("The kubeaidConfig section is missing from the config")?
                  .url
                  .try_into()?;
    let kubeaid_config = KubeAidConfig::new(kubeaid_config_url, ssh_access.as_ref())?;

    Ok(Self { ssh_access,
              kubeaid,
              kubeaid_config })
  }
}

pub struct SSHAccess {
  pub known_hosts: Vec<String>,
  pub username:    String,
  pub method:      SSHAccessMethod
}

impl TryFrom<raw::SSHAccess> for SSHAccess {
  type Error = Box<dyn std::error::Error>;

  fn try_from(ssh_access: raw::SSHAccess) -> Result<Self, Self::Error> {
    let username = ssh_access.username.unwrap_or(DEFAULT_GIT_SSH_USERNAME.into());

    let method = match ssh_access.private_key_file_path {
      | Some(private_key_file_path) => SSHAccessMethod::PrivateKey(private_key_file_path.try_into()?),

      | None if ssh_agent_available() => SSHAccessMethod::Agent,

      | _ =>
        return Err("Neither SSH private key, nor SSH auth socket (for using SSH agent) provided".into()),
    };

    Ok(Self { known_hosts: ssh_access.known_hosts,
              username,
              method })
  }
}

fn ssh_agent_available() -> bool {
  match std::env::var_os("SSH_AUTH_SOCK") {
    | Some(socket) if !socket.is_empty() => UnixStream::connect(&socket).is_ok(),

    | _ => false
  }
}

pub enum SSHAccessMethod {
  Agent,
  PrivateKey(SSHKeyPair)
}

pub struct SSHKeyPair {
  pub private_key_file_path: PathBuf,
  pub private_key:           String,

  pub public_key:  String,
  pub fingerprint: String
}

impl TryFrom<PathBuf> for SSHKeyPair {
  type Error = Box<dyn std::error::Error>;

  fn try_from(private_key_file_path: PathBuf) -> Result<Self, Self::Error> {
    let private_key = fs::read_to_string(&private_key_file_path)?;
    let parsed_private_key = ssh_key::PrivateKey::from_openssh(private_key.as_bytes())?;

    let public_key = parsed_private_key.public_key();
    let fingerprint = legacy_md5_fingerprint(public_key)?;

    Ok(Self { private_key_file_path,
              private_key,

              public_key: public_key.to_string(),
              fingerprint })
  }
}

fn legacy_md5_fingerprint(public_key: &ssh_key::PublicKey)
                          -> Result<String, Box<dyn std::error::Error>> {
  let digest = md5::compute(public_key.to_bytes()?);
  Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":"))
}

pub struct KubeAid {
  pub url:     RepositoryURL,
  pub version: KubeAidVersion
}

impl KubeAid {
  /// Constructs a [`KubeAid`], resolving its version : if a version is provided, we verify that
  /// such a version tag exists in the repository. Otherwise, we fall back to the repository's
  /// latest version tag.
  ///
  /// When the repository is private, SSH access details must be provided, since its version tags
  /// can then only be resolved over SSH.
  pub fn new(url: RepositoryURL,
             version: Option<String>,
             ssh_access: Option<&SSHAccess>)
             -> Result<Self, Box<dyn std::error::Error>> {
    let version = match version {
      | Some(version) => KubeAidVersion::new(&url, version, ssh_access)?,

      | None => KubeAidVersion::latest(&url, ssh_access)?,
    };

    Ok(Self { url,
              version })
  }
}

pub struct KubeAidVersion(pub String);

impl KubeAidVersion {
  /// Constructs a [`KubeAidVersion`], ensuring that a git tag named `version` actually exists in
  /// the KubeAid repository at `repository_url`.
  pub fn new(repository_url: &RepositoryURL,
             version: String,
             ssh_access: Option<&SSHAccess>)
             -> Result<Self, Box<dyn std::error::Error>> {
    if !fetch_tags(repository_url, ssh_access)?.iter().any(|tag| tag == &version) {
      return Err(format!("Version tag `{version}` doesn't exist in the KubeAid repository at {}",
                         repository_url.as_url().to_bstring()).into());
    }

    Ok(Self(version))
  }

  /// Constructs a [`KubeAidVersion`] from the latest version tag in the KubeAid repository at
  /// `repository_url`. Since the tag is read straight from the repository, its existence needn't be
  /// verified.
  pub fn latest(repository_url: &RepositoryURL,
                ssh_access: Option<&SSHAccess>)
                -> Result<Self, Box<dyn std::error::Error>> {
    let latest_tag = fetch_tags(repository_url, ssh_access)?
      .into_iter()
      .max_by_key(|tag| version_sort_key(tag))
      .ok_or_else(|| {
        format!("KubeAid repository at {} has no version tags to select a latest version from",
                repository_url.as_url().to_bstring())
      })?;

    Ok(Self(latest_tag))
  }
}

pub struct KubeAidConfig {
  pub url: RepositoryURL
}

impl KubeAidConfig {
  /// Constructs a [`KubeAidConfig`].
  ///
  /// A public repository has already been verified to be reachable while constructing the
  /// [`RepositoryHTTPsURL`]. A private repository's reachability, however, is verified over SSH,
  /// so SSH access details must then be provided.
  pub fn new(url: RepositoryURL,
             ssh_access: Option<&SSHAccess>)
             -> Result<Self, Box<dyn std::error::Error>> {
    match &url {
      | RepositoryURL::Public(_) => Ok(Self { url }),

      | RepositoryURL::Private(ssh_url) => {
        let ssh_access =
          ssh_access.ok_or("SSH access details must be provided, since the KubeAid config repository is private")?;

        // The repository is reachable iff its refs can be fetched over SSH using the given
        // authentication details.
        fetch_refs_over_ssh(ssh_url, ssh_access)?;

        Ok(Self { url })
      }
    }
  }
}

pub enum RepositoryURL {
  Public(RepositoryHTTPsURL),
  Private(RepositorySSHURL)
}

impl RepositoryURL {
  /// Returns the underlying [`Url`], whichever variant holds it.
  pub fn as_url(&self) -> &Url {
    match self {
      | Self::Public(RepositoryHTTPsURL(url)) | Self::Private(RepositorySSHURL(url)) => url
    }
  }
}

impl TryFrom<String> for RepositoryURL {
  type Error = Box<dyn std::error::Error>;

  fn try_from(url: String) -> Result<Self, Self::Error> {
    let parsed = gix_url::parse(url.as_str().into())?;

    Ok(match parsed.scheme {
         | Scheme::Http | Scheme::Https => Self::Public(RepositoryHTTPsURL::new(parsed)?),

         | Scheme::Ssh => Self::Private(RepositorySSHURL(parsed)),

         | _ => return Err("Unsupported git repository URL scheme".into())
       })
  }
}

pub struct RepositoryHTTPsURL(pub Url);

impl RepositoryHTTPsURL {
  pub fn new(url: Url) -> Result<Self, Box<dyn std::error::Error>> {
    let url_as_string = url.to_bstring().to_string();

    let info_refs_url = git_upload_pack_info_refs_url(&url);

    let response = ureq::get(&info_refs_url)
      .call()
      .map_err(|error| format!("Git repository at {url_as_string} isn't publicly reachable : {error}"))?;

    match response.status() {
      | 200 => Ok(Self(url)),

      | _ =>
        Err(format!("Git repository at {url_as_string} isn't publicly reachable (HTTP status {})",
                    response.status()).into()),
    }
  }
}

pub struct RepositorySSHURL(pub Url);

/// Builds the git smart-HTTP ref advertisement URL (`<url>/info/refs?service=git-upload-pack`) for
/// the given repository URL. A `GET` against it lists the repository's refs and succeeds only when
/// the repository is readable anonymously.
fn git_upload_pack_info_refs_url(url: &Url) -> String {
  format!("{}/info/refs?service=git-upload-pack",
          url.to_bstring().to_string().trim_end_matches('/'))
}

/// Fetches the git tag names advertised by the repository at `repository_url`, without cloning it :
/// over the smart-HTTP protocol when the repository is public, and over SSH when it's private (SSH
/// access details must then be provided).
fn fetch_tags(repository_url: &RepositoryURL,
              ssh_access: Option<&SSHAccess>)
              -> Result<Vec<String>, Box<dyn std::error::Error>> {
  let refs_advertisement = match repository_url {
    | RepositoryURL::Public(https_url) => fetch_refs_over_https(&https_url.0)?,

    | RepositoryURL::Private(ssh_url) => {
      let ssh_access = ssh_access.ok_or_else(|| {
                         format!("SSH access details must be provided, since the repository at {} is private",
                                 repository_url.as_url().to_bstring())
                       })?;

      fetch_refs_over_ssh(ssh_url, ssh_access)?
    }
  };

  // Each advertised ref sits on its own line as `<object-hash> refs/tags/<name>`, and annotated
  // tags additionally advertise a peeled `refs/tags/<name>^{}` entry which we fold back onto the
  // tag name.
  let mut tags = refs_advertisement.split(['\n', '\0'])
                                   .filter_map(|line| line.split_once(" refs/tags/"))
                                   .map(|(_, tag)| tag.trim_end_matches("^{}").to_string())
                                   .collect::<Vec<_>>();
  tags.dedup();

  Ok(tags)
}

/// Fetches the git ref advertisement of the repository at `url` over the smart-HTTP protocol.
fn fetch_refs_over_https(url: &Url) -> Result<String, Box<dyn std::error::Error>> {
  let info_refs_url = git_upload_pack_info_refs_url(url);

  Ok(ureq::get(&info_refs_url)
       .call()
       .map_err(|error| {
         format!("Couldn't fetch refs from the repository at {} : {error}", url.to_bstring())
       })?
       .into_string()?)
}

/// Fetches the git ref advertisement of the private repository at `repository_url`, by running
/// `git-upload-pack` on the remote through the `ssh` program (exactly how git / gix itself talks to
/// an SSH remote), authenticated using the given SSH access details.
fn fetch_refs_over_ssh(repository_url: &RepositorySSHURL,
                       ssh_access: &SSHAccess)
                       -> Result<String, Box<dyn std::error::Error>> {
  let url = &repository_url.0;

  // Using the argument-safe accessors guards against a malicious URL part being interpreted as an
  // option by ssh / the remote shell.
  let host = url.host_argument_safe().ok_or("SSH repository URL has a missing / argument-unsafe host")?;
  let repository_path = url.path_argument_safe()
                           .ok_or("SSH repository URL has an argument-unsafe path")?
                           .to_string();
  if repository_path.contains('\'') {
    return Err("SSH repository URL path mustn't contain single quotes".into());
  }

  let username = match url.user() {
    | Some(_) => url.user_argument_safe().ok_or("SSH repository URL has an argument-unsafe username")?,

    | None => &ssh_access.username
  };

  // ssh only reads known hosts from a file, so materialize the configured entries into a temporary
  // one.
  let known_hosts_file_path =
    std::env::temp_dir().join(format!("kubeaid-cli-known-hosts-{}", std::process::id()));
  fs::write(&known_hosts_file_path, ssh_access.known_hosts.join("\n") + "\n")?;

  let mut ssh = Command::new("ssh");
  ssh.arg("-o").arg("BatchMode=yes") // Fail instead of prompting for input.
     .arg("-o").arg("StrictHostKeyChecking=yes")
     .arg("-o").arg(format!("UserKnownHostsFile={}", known_hosts_file_path.display()));

  match &ssh_access.method {
    // The SSH agent (reachable at SSH_AUTH_SOCK) is consulted by ssh by default.
    | SSHAccessMethod::Agent => {},

    | SSHAccessMethod::PrivateKey(key_pair) => {
      ssh.arg("-o").arg("IdentitiesOnly=yes")
         .arg("-i").arg(&key_pair.private_key_file_path);
    }
  }

  if let Some(port) = url.port {
    ssh.arg("-p").arg(port.to_string());
  }

  let mut ssh_process = ssh.arg(format!("{username}@{host}"))
                           .arg(format!("git-upload-pack '{repository_path}'"))
                           .stdin(Stdio::piped())
                           .stdout(Stdio::piped())
                           .stderr(Stdio::piped())
                           .spawn()?;

  // After advertising its refs, git-upload-pack waits for us to request objects. Sending a flush
  // packet instead tells it we want nothing, making it exit gracefully.
  if let Some(mut stdin) = ssh_process.stdin.take() {
    let _ = stdin.write_all(b"0000");
  }

  let output = ssh_process.wait_with_output();
  let _ = fs::remove_file(&known_hosts_file_path);
  let output = output?;

  if !output.status.success() {
    return Err(format!("Couldn't fetch refs from the repository at {} over SSH : {}",
                       url.to_bstring(),
                       String::from_utf8_lossy(&output.stderr).trim()).into());
  }

  Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Produces a coarse ordering key for a version tag from its numeric components (for example
/// `v1.2.10` -> `[1, 2, 10]`). This is a heuristic: it ignores any pre-release suffix, so a
/// pre-release tag sorts as if it were the final release.
fn version_sort_key(tag: &str) -> Vec<u64> {
  tag.split(|character: char| !character.is_ascii_digit())
     .filter_map(|component| component.parse().ok())
     .collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hydration_works() -> Result<(), Box<dyn std::error::Error>> {
    let unparsed = "
      repositories:
        kubeaidConfig:
          url: https://github.com/Archisman-Mridha/kubeaid-config
    ";

    let parsed = serde_yaml::from_str::<raw::Config>(unparsed)?;

    let hydrated_config: Config = parsed.try_into()?;
    let repositories = &hydrated_config.repositories;

    assert!(repositories.ssh_access.is_none());

    // The KubeAid repository falls back to the public upstream, with its latest version tag
    // resolved.
    assert!(matches!(repositories.kubeaid.url, RepositoryURL::Public(_)));
    assert!(!repositories.kubeaid.version.0.is_empty());

    assert!(matches!(repositories.kubeaid_config.url, RepositoryURL::Public(_)));

    Ok(())
  }
}
