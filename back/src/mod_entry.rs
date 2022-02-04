use core::fmt;
use std::{fs::File, io::Read};

use ferinth::structures::version_structs::ModLoader as FeModLoader;
use lazy_static::lazy_static;
use mc_mod_meta::{
    common::MinecraftMod, fabric::FabricManifest, forge::ForgeManifest, ModLoader as McModLoader,
};
use regex::Regex;
use sha2::Digest;

#[derive(Clone, Debug)]
pub struct ModEntry {
    pub id: String,
    pub version: String,
    pub display_name: String,
    pub modloader: ModLoader,
    pub hashes: Hashes,
    pub modrinth_data: Option<ModrinthData>,
    pub state: FileState,
    pub sourced_from: Source,
}

// Middleman "ModLoader" enum to convert between those of the other crates
#[derive(Clone, Debug)]
pub enum ModLoader {
    Forge,
    Fabric,
}

impl fmt::Display for ModLoader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl From<ModLoader> for FeModLoader {
    fn from(modloader: ModLoader) -> Self {
        match modloader {
            ModLoader::Forge => Self::Forge,
            ModLoader::Fabric => Self::Fabric,
        }
    }
}

impl From<McModLoader> for ModLoader {
    fn from(modloader: McModLoader) -> Self {
        match modloader {
            McModLoader::Forge => Self::Forge,
            McModLoader::Fabric => Self::Fabric,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ModrinthData {
    pub id: String,
    pub latest_valid_version: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Hashes {
    pub sha1: String,
    pub sha512: String,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FileState {
    Current,
    Outdated,
    Invalid,
    Local,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Source {
    Local,
    ExplicitLocal,
    Modrinth,
    CurseForge,
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl ModEntry {
    fn new(mc_mod: MinecraftMod, hashes: &Hashes, modrinth_data: Option<ModrinthData>) -> Self {
        let MinecraftMod {
            id,
            version,
            display_name,
            modloader,
        } = mc_mod;

        Self {
            id,
            version,
            display_name,
            modloader: modloader.into(),
            hashes: hashes.clone(),
            modrinth_data,
            state: FileState::Local,
            sourced_from: Source::Local,
        }
    }

    pub fn from_file(mut file: File) -> Vec<Self> {
        let metadata = file.metadata().unwrap();
        let mut buf = vec![0; metadata.len() as usize];

        std::fs::File::read(&mut file, &mut buf).unwrap();

        let sha1 = hex::encode(sha1::Sha1::digest(&buf));
        let sha512 = hex::encode(sha2::Sha512::digest(&buf));
        let hashes = Hashes { sha1, sha512 };

        let mut mod_vec = Vec::new();

        match mc_mod_meta::get_modloader(&file) {
            Ok(modloader) => match modloader {
                mc_mod_meta::ModLoader::Forge => {
                    let forge_meta = ForgeManifest::from_file(file).unwrap();
                    for mod_meta in forge_meta.mods {
                        let mc_mod = MinecraftMod::from(mod_meta);
                        mod_vec.push(Self::new(mc_mod, &hashes, None));
                    }
                }
                mc_mod_meta::ModLoader::Fabric => {
                    let mod_meta = FabricManifest::from_file(file).unwrap();
                    let mc_mod = MinecraftMod::from(mod_meta);
                    mod_vec.push(Self::new(mc_mod, &hashes, None));
                }
            },
            Err(err) => {
                println!("{}", err);
            }
        }

        mod_vec
    }

    pub fn normalized_version(&self) -> String {
        lazy_static! {
            static ref VERSION_REGEX: Regex = Regex::new("[0-9]+\\.[0-9]+\\.[0-9]+").unwrap();
        };

        let version: String = VERSION_REGEX.captures(self.version.as_str()).map_or_else(
            || self.version.clone(),
            |matches| matches.get(0).unwrap().as_str().to_string(),
        );

        version
    }
}