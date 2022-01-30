use std::{
    collections::HashMap,
    sync::mpsc::{Receiver, Sender},
};

use ferinth::Ferinth;
use tokio::runtime::Runtime;

use crate::{
    message::{FetchingModContext, Message},
    mod_entry::{FileState, ModEntry, Source},
};

pub mod message;
pub mod mod_entry;

pub struct Back {
    back_tx: Sender<Message>,
    front_rx: Receiver<Message>,
    rt: Runtime,
    modrinth: Ferinth,
}

impl Back {
    pub fn new(back_tx: Sender<Message>, front_rx: Receiver<Message>) -> Self {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let modrinth = Ferinth::new("Still a test app");

        Self {
            back_tx,
            front_rx,
            rt,
            modrinth,
        }
    }
    pub fn init(&self) {
        self.rt.block_on(async {
            loop {
                match self.front_rx.recv() {
                    Ok(message) => match message {
                        Message::UpdateModList {
                            mod_list,
                            mod_hash_cache,
                        } => {
                            self.update_mod_list(mod_list, mod_hash_cache).await;
                        }
                        _ => unreachable!(),
                    },
                    Err(_err) => {
                        //TODO: Handle
                    }
                }
            }
        });
    }

    async fn update_mod_list(
        &self,
        mut mod_list: Vec<ModEntry>,
        mut mod_hash_cache: HashMap<String, String>,
    ) {
        let list_length = mod_list.len();

        for (position, entry) in mod_list.iter_mut().enumerate() {
            self.back_tx
                .send(Message::FetchingMod {
                    context: FetchingModContext {
                        name: entry.display_name.clone(),
                        position,
                        total: list_length,
                    },
                })
                .unwrap();

            entry.modrinth_id = if let Some(id) = mod_hash_cache.get(&entry.hashes.sha1) {
                Some(id.to_owned())
            } else {
                if let Some(modrinth_id) =
                    get_modrinth_id(&self.modrinth, entry.hashes.sha1.as_str()).await
                {
                    mod_hash_cache.insert(entry.hashes.sha1.to_owned(), modrinth_id.to_owned());
                    Some(modrinth_id)
                } else {
                    None
                }
            };

            if let Some(modrinth_id) = &entry.modrinth_id {
                match self.modrinth.list_versions(modrinth_id.as_str()).await {
                    Ok(version_data) => {
                        entry.sourced_from = Source::Modrinth;
                        // Assume its outdated unless proven otherwise
                        entry.state = FileState::Outdated;

                        'outer: for file in &version_data[0].files {
                            if let Some(hash) = &file.hashes.sha1 {
                                if hash == &entry.hashes.sha1 {
                                    entry.state = FileState::Current;
                                    break 'outer;
                                }
                            }
                        }
                    }
                    Err(err) => {
                        dbg!(err);
                        entry.state = FileState::Local
                    }
                };
            }
        }

        async fn get_modrinth_id(modrinth: &Ferinth, mod_hash: &str) -> Option<String> {
            match modrinth.get_version_from_file_hash(mod_hash).await {
                Ok(result) => Some(result.mod_id),
                Err(_err) => None,
            }
        }

        self.back_tx
            .send(Message::UpdateModList {
                mod_list,
                mod_hash_cache,
            })
            .unwrap();
    }
}
