use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::mpsc::{Receiver, Sender},
};

use bytes::Bytes;
use hash::Hashes;
use messages::{CheckProgress, ToBackend, ToFrontend};
use mod_entry::{ModEntry, ModLoader};
use modrinth::Modrinth;
mod minecraft_path;
mod modrinth;

mod error;
mod hash;
pub mod messages;
pub mod mod_entry;

pub use daedalus::minecraft::Version as GameVersion;
use tracing::{debug, error, info};

pub struct Back {
    mod_list: Vec<ModEntry>,
    folder_path: PathBuf,
    modrinth: Modrinth,
    back_tx: Sender<ToFrontend>,
    front_rx: Receiver<ToBackend>,
    egui_epi_frame: Option<epi::Frame>,
}

impl Back {
    pub fn new(
        folder_path: Option<PathBuf>,
        back_tx: Sender<ToFrontend>,
        front_rx: Receiver<ToBackend>,
        egui_epi_frame: Option<epi::Frame>,
    ) -> Self {
        Self {
            mod_list: Default::default(),
            folder_path: folder_path.unwrap_or_else(minecraft_path::default_mod_dir),
            modrinth: Default::default(),
            back_tx,
            front_rx,
            egui_epi_frame,
        }
    }

    pub fn init(&mut self) {
        info!("Initializing backend");

        let rt = tokio::runtime::Runtime::new().unwrap();
        debug!("Runtime created");

        rt.block_on(async {
            loop {
                match self.front_rx.recv() {
                    Ok(message) => {
                        match message {
                            ToBackend::ScanFolder => {
                                self.scan_folder();

                                self.sort_and_send_list();
                            }

                            ToBackend::CheckForUpdates { game_version } => {
                                self.scan_folder();

                                self.check_for_updates(game_version).await;

                                self.sort_and_send_list();
                            }

                            ToBackend::GetVersionMetadata => {
                                match daedalus::minecraft::fetch_version_manifest(None).await {
                                    Ok(manifest) => self
                                        .back_tx
                                        .send(ToFrontend::SetVersionMetadata { manifest })
                                        .unwrap(),
                                    Err(err) => self
                                        .back_tx
                                        .send(ToFrontend::BackendError { error: err.into() })
                                        .unwrap(),
                                };
                            }

                            ToBackend::UpdateMod { mod_entry } => {
                                self.update_mod(mod_entry).await;
                            }

                            ToBackend::AddMod {
                                modrinth_id,
                                game_version,
                                modloader,
                            } => {
                                self.add_mod(modrinth_id, game_version, modloader).await;
                            }
                        }

                        if let Some(frame) = &self.egui_epi_frame {
                            frame.request_repaint();
                        }
                    }
                    Err(error) => {
                        error!(%error, "There was an error when receiving a message from the frontend:");
                    }
                };
            }
        });
    }

    fn sort_and_send_list(&mut self) {
        info!(length = self.mod_list.len(), "Sending the mods list");
        self.mod_list
            .sort_by(|entry_1, entry_2| entry_1.display_name.cmp(&entry_2.display_name));

        self.back_tx
            .send(ToFrontend::UpdateModList {
                mod_list: self.mod_list.clone(),
            })
            .unwrap();
    }

    fn scan_folder(&mut self) {
        info!(folder_path = %self.folder_path.display(), "Scanning the mods folder");

        let old_list = self.mod_list.clone();

        self.mod_list.clear();

        let read_dir = fs::read_dir(&self.folder_path).unwrap();

        'file_loop: for file_entry in read_dir {
            let path = file_entry.unwrap().path();

            // Minecraft does not really care about mods within folders, therefore skip anything that is not a file
            if path.is_file() {
                debug!(?path, "Parsing file");
                let mut file = fs::File::open(&path).unwrap();

                match ModEntry::from_file(&mut file, Some(path.clone())) {
                    Ok(mut entry) => self.mod_list.append(&mut entry),
                    Err(error) => {
                        // In the case of an error the mod list will be cleared
                        self.mod_list.clear();

                        self.back_tx
                            .send(ToFrontend::BackendErrorContext {
                                msg: format!("Could not parse: {}", path.display()),
                            })
                            .unwrap();
                        self.back_tx
                            .send(ToFrontend::BackendError { error })
                            .unwrap();
                        break 'file_loop;
                    }
                }
            }
        }

        // Ensure the important bits are kept from the old list
        for mod_entry in self.mod_list.iter_mut() {
            let filtered_old: Vec<&ModEntry> = old_list
                .iter()
                .filter(|filter_entry| filter_entry.id == mod_entry.id)
                .collect();

            if !filtered_old.is_empty() {
                mod_entry.sourced_from = filtered_old[0].sourced_from;
                mod_entry.modrinth_data = filtered_old[0].modrinth_data.clone();

                // If the file has not changed, the state can also be kept
                if mod_entry.hashes.sha1 == filtered_old[0].hashes.sha1 {
                    mod_entry.state = filtered_old[0].state;
                }
            }
        }
    }

    async fn check_for_updates(&mut self, game_version: String) {
        let total_len = self.mod_list.len();
        for (position, mod_entry) in self.mod_list.iter_mut().enumerate() {
            // Update the frontend on whats happening
            self.back_tx
                .send(ToFrontend::CheckForUpdatesProgress {
                    progress: CheckProgress {
                        name: mod_entry.display_name.clone(),
                        position,
                        total_len,
                    },
                })
                .unwrap();

            if let Some(frame) = &self.egui_epi_frame {
                frame.request_repaint();
            }

            if let Err(error) = self
                .modrinth
                .check_for_updates(mod_entry, &game_version)
                .await
            {
                self.back_tx
                    .send(ToFrontend::BackendError { error })
                    .unwrap();
            };
        }
    }

    async fn update_mod(&mut self, mod_entry: ModEntry) {
        info!(
            entry_name = %mod_entry.display_name,
            path = ?mod_entry.path,
            sha1 = %mod_entry.hashes.sha1,
            "Updating mod"
        );
        if let Ok(bytes) = self.modrinth.update_mod(&mod_entry).await {
            debug!("Update downloaded");
            let read_dir = fs::read_dir(&self.folder_path).unwrap();

            'file_loop: for file_entry in read_dir {
                let path = file_entry.unwrap().path();

                if path.is_file() {
                    let mut file = fs::File::open(&path).unwrap();

                    let hashes = Hashes::get_hashes_from_file(&mut file).unwrap();

                    // We found the file the mod_entry belongs to
                    if mod_entry.hashes.sha1 == hashes.sha1 {
                        std::fs::remove_file(path).unwrap();

                        self.create_mod_file(&mod_entry, &bytes);
                        break 'file_loop;
                    }
                }
            }
        };
    }

    async fn add_mod(&mut self, modrinth_id: String, game_version: String, modloader: ModLoader) {
        match self
            .modrinth
            .create_mod_entry(modrinth_id, game_version, modloader)
            .await
        {
            Ok(mod_entry) => match self.modrinth.update_mod(&mod_entry).await {
                Ok(bytes) => {
                    self.create_mod_file(&mod_entry, &bytes);
                }
                Err(error) => self
                    .back_tx
                    .send(ToFrontend::BackendError { error })
                    .unwrap(),
            },
            Err(error) => {
                self.back_tx
                    .send(ToFrontend::BackendError { error })
                    .unwrap();
            }
        };
    }

    fn create_mod_file(&mut self, mod_entry: &ModEntry, bytes: &Bytes) {
        info!(
            entry_name = %mod_entry.display_name,
            "Creating a new mod file"
        );
        // The data is guaranteed to exist, unwrapping here is fine
        let path = self.folder_path.join(
            &mod_entry
                .modrinth_data
                .as_ref()
                .unwrap()
                .latest_valid_version
                .as_ref()
                .unwrap()
                .filename,
        );

        // Essentially fs::File::create(path) but with read access as well
        let mut new_mod_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();

        new_mod_file.write_all(bytes).unwrap();

        let mut new_entries = ModEntry::from_file(&mut new_mod_file, Some(path)).unwrap();

        for new_mod_entry in new_entries.iter_mut() {
            // Ensure the data for the entry is kept
            new_mod_entry.modrinth_data = mod_entry.modrinth_data.clone();
            new_mod_entry.sourced_from = mod_entry.sourced_from;

            for list_entry in self.mod_list.iter_mut() {
                // The hash has to be compared to the old entry, the slug/id can be compared to the new one
                if list_entry.hashes.sha1 == mod_entry.hashes.sha1
                    && list_entry.id == new_mod_entry.id.clone()
                {
                    *list_entry = new_mod_entry.clone();
                }
            }
        }

        self.scan_folder();

        self.sort_and_send_list();
    }
}
