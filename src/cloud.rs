use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

use crate::utils::get_maven_artifact_path;

pub const LAUNCHER_API: &str = "https://api.liquidbounce.net";

pub struct LauncherApi;

impl LauncherApi {
    pub(crate) async fn load_branches() -> Result<Vec<String>> {
        Ok(reqwest::get(format!("{}/api/v1/version/branches", LAUNCHER_API)).await?.error_for_status()?.json::<Vec<String>>().await?)
    }

    pub(crate) async fn load_all_builds() -> Result<Vec<Build>> {
        Ok(reqwest::get(format!("{}/api/v1/version/builds", LAUNCHER_API)).await?.error_for_status()?.json::<Vec<Build>>().await?)
    }

    pub(crate) async fn load_builds(branch: String) -> Result<Vec<Build>> {
        Ok(reqwest::get(format!("{}/api/v1/version/builds/{}", LAUNCHER_API, branch)).await?.error_for_status()?.json::<Vec<Build>>().await?)
    }

    pub(crate) async fn load_version_manifest(build_id: u32) -> Result<LaunchManifest> {
        Ok(reqwest::get(format!("{}/api/v1/version/launch/{}", LAUNCHER_API, build_id)).await?.error_for_status()?.json::<LaunchManifest>().await?)
    }
}

#[derive(Debug, Deserialize)]
pub struct Build {
    pub build_id: u32,
    pub commit_id: String,
    pub branch: String,
    pub lb_version: String,
    pub mc_version: String,
    // pub date: DateTime<Local>,
    pub message: String,
    pub url: String,
    pub fabric_api_version: String,
    pub fabric_loader_version: String,
    pub kotlin_version: String,
    pub kotlin_mod_version: String
}

#[derive(Deserialize, Debug)]
pub struct LaunchManifest {
    pub build: Build,
    pub loader: LoaderVersion,
    pub mods: Vec<LoaderMod>,
    pub repositories: BTreeMap<String, String>,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ModSource {
    #[serde(rename = "skip")]
    #[serde(rename_all = "camelCase")]
    SkipAd { artifact_name: String, url: String, #[serde(default)] extract: bool },
    #[serde(rename = "repository")]
    #[serde(rename_all = "camelCase")]
    Repository { repository: String, artifact: String },
}

impl ModSource {
    pub fn get_path(&self) -> anyhow::Result<String> {
        Ok(
            match self {
                ModSource::SkipAd { artifact_name, .. } => format!("{}.jar", artifact_name),
                ModSource::Repository { repository, artifact } => get_maven_artifact_path(artifact)?
            }
        )
    }
}

#[derive(Deserialize, Debug)]
pub struct LoaderMod {
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub default: bool,
    pub name: String,
    pub source: ModSource,
}


#[derive(Deserialize, Debug)]
pub struct LoaderVersion {
    pub subsystem: LoaderSubsystem,
    pub launcher_manifest: String,
    pub mod_directory: String,
}

#[derive(Deserialize, Debug)]
pub enum LoaderSubsystem {
    #[serde(rename = "fabric")]
    Fabric,
    #[serde(rename = "forge")]
    Forge,
}