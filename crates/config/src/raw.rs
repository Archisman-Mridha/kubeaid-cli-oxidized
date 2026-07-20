use {serde::Deserialize, std::path::PathBuf};

#[derive(Debug, Deserialize, prompt::Prompt)]
#[serde(rename_all = "camelCase")]
pub struct Config {
  // Optional only so that deserialization tolerates its absence : the wizard / hydration treat it
  // as required.
  #[prompt(required)]
  pub repositories: Option<Repositories>
}

#[derive(Debug, Deserialize, prompt::Prompt)]
#[serde(rename_all = "camelCase")]
pub struct Repositories {
  pub ssh_access: Option<SSHAccess>,

  pub kubeaid: Option<KubeAid>,

  // Optional only so that deserialization tolerates its absence : the wizard / hydration treat it
  // as required.
  #[prompt(required)]
  pub kubeaid_config: Option<KubeAidConfig>
}

#[derive(Debug, Deserialize, prompt::Prompt)]
#[serde(rename_all = "camelCase")]
pub struct SSHAccess {
  pub known_hosts:           Vec<String>,
  pub username:              Option<String>,
  pub private_key_file_path: Option<PathBuf>
}

#[derive(Debug, Deserialize, prompt::Prompt)]
#[serde(rename_all = "camelCase")]
pub struct KubeAid {
  pub url:     Option<String>,
  pub version: Option<String>
}

#[derive(Debug, Deserialize, prompt::Prompt)]
#[serde(rename_all = "camelCase")]
pub struct KubeAidConfig {
  pub url: String
}

#[cfg(test)]
mod tests {
  use {super::*,
       prompt::{Prompt, ScriptedPrompter}};

  #[test]
  fn prompting_fills_missing_values() -> Result<(), Box<dyn std::error::Error>> {
    // The config file only declares an (empty) repositories section.
    let parsed = serde_yaml::from_str::<Config>("repositories: {}")?;

    let mut prompter = ScriptedPrompter::new([
      "n",                                                   // Don't configure repositories.sshAccess.
      "y",                                                   // Configure repositories.kubeaid.
      "https://github.com/Obmondo/KubeAid",                  // repositories.kubeaid.url.
      "",                                                    // repositories.kubeaid.version : skipped.
      "https://github.com/Archisman-Mridha/kubeaid-config",  // repositories.kubeaidConfig.url.
    ]);
    let completed = Config::prompt_or_keep(Some(parsed), &mut prompter, "")?;

    let repositories = completed.repositories.ok_or("repositories section missing")?;

    assert!(repositories.ssh_access.is_none());

    let kubeaid = repositories.kubeaid.ok_or("kubeaid section missing")?;
    assert_eq!(kubeaid.url.as_deref(), Some("https://github.com/Obmondo/KubeAid"));
    assert!(kubeaid.version.is_none());

    let kubeaid_config = repositories.kubeaid_config.ok_or("kubeaidConfig section missing")?;
    assert_eq!(kubeaid_config.url, "https://github.com/Archisman-Mridha/kubeaid-config");

    Ok(())
  }

  #[test]
  fn prompting_keeps_existing_values() -> Result<(), Box<dyn std::error::Error>> {
    let unparsed = "
      repositories:
        kubeaidConfig:
          url: https://github.com/Archisman-Mridha/kubeaid-config
    ";
    let parsed = serde_yaml::from_str::<Config>(unparsed)?;

    let mut prompter = ScriptedPrompter::new([
      "n", // Don't configure repositories.sshAccess.
      "n", // Don't configure repositories.kubeaid.
    ]);
    let completed = Config::prompt_or_keep(Some(parsed), &mut prompter, "")?;

    // The value from the config file is kept without re-prompting for it.
    let repositories = completed.repositories.ok_or("repositories section missing")?;
    assert_eq!(repositories.kubeaid_config.ok_or("kubeaidConfig section missing")?.url,
               "https://github.com/Archisman-Mridha/kubeaid-config");

    Ok(())
  }
}
