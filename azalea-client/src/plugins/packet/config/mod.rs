mod events;

use std::io::Cursor;

use azalea_entity::LocalEntity;
use azalea_protocol::{
    packets::{ConnectionProtocol, config::*},
    read::{ReadPacketError, deserialize_packet},
};
use bevy_ecs::prelude::*;
pub use events::*;
use tracing::{debug, warn};

use super::{as_system, declare_packet_handlers};
use crate::{
    client::InConfigState,
    connection::RawConnection,
    cookies::{RequestCookieEvent, StoreCookieEvent},
    disconnect::DisconnectEvent,
    local_player::WorldHolder,
    packet::game::{KeepAliveEvent, ResourcePackEvent},
};

pub fn process_raw_packet(
    ecs: &mut World,
    player: Entity,
    raw_packet: &[u8],
) -> Result<(), Box<ReadPacketError>> {
    let packet = deserialize_packet(&mut Cursor::new(raw_packet))?;
    process_packet(ecs, player, &packet);
    Ok(())
}

pub fn process_packet(ecs: &mut World, player: Entity, packet: &ClientboundConfigPacket) {
    let mut handler = ConfigPacketHandler { player, ecs };

    declare_packet_handlers!(
        ClientboundConfigPacket,
        packet,
        handler,
        [
            cookie_request,
            custom_payload,
            disconnect,
            finish_configuration,
            keep_alive,
            ping,
            reset_chat,
            registry_data,
            resource_pack_pop,
            resource_pack_push,
            store_cookie,
            transfer,
            update_enabled_features,
            update_tags,
            select_known_packs,
            custom_report_details,
            server_links,
            clear_dialog,
            show_dialog,
            code_of_conduct,
        ]
    );
}

pub struct ConfigPacketHandler<'a> {
    pub ecs: &'a mut World,
    pub player: Entity,
}
impl ConfigPacketHandler<'_> {
    pub fn registry_data(&mut self, p: &ClientboundRegistryData) {
        as_system::<Query<&WorldHolder>>(self.ecs, |mut query| {
            let world_holder = query.get_mut(self.player).unwrap();
            let mut world = world_holder.shared.write();

            // add the new registry data
            world
                .registries
                .append(p.registry_id.clone(), p.entries.clone());
        });
    }

    pub fn custom_payload(&mut self, p: &ClientboundCustomPayload) {
        let channel_name = p.identifier.to_string();
        tracing::info!("CUSTOM PAYLOAD RECEIVED on channel: {}", channel_name);

        // Handle Fabric API registry sync synchronously to avoid being disconnected
        // before we can respond.
        if channel_name == "fabric:registry/sync" {
            tracing::info!(
                "Fabric registry sync received ({} bytes), sending completion acknowledgment",
                p.data.len()
            );
            use azalea_registry::identifier::Identifier;
            self.ecs
                .commands()
                .trigger(SendConfigPacketEvent::new(
                    self.player,
                    ServerboundCustomPayload {
                        identifier: Identifier::new("fabric:registry/sync/complete"),
                        data: vec![].into(),
                    },
                ));
            tracing::info!("Fabric registry sync completion sent");
        }

        // Handle Cardinal Components API entity sync packets.
        // We just acknowledge receipt by doing nothing - the bot doesn't need component data.
        if channel_name == "cardinal-components:entity_sync" {
            tracing::info!(
                "CONFIG: Received cardinal-components:entity_sync ({} bytes), reading packet data",
                p.data.len()
            );
            // Try to read the packet data to show we can handle it
            // Format: entity_id (varint) + component_data
            if p.data.len() >= 1 {
                tracing::info!(
                    "  Packet data (first 32 bytes): {:?}",
                    &p.data[..p.data.len().min(32)]
                );
            }
        }

        // When we receive minecraft:register, we need to respond by registering
        // the Fabric API channels so the server knows we support them.
        if channel_name == "minecraft:register" {
            tracing::info!("Received minecraft:register, registering Fabric and CCA channels");
            use azalea_registry::identifier::Identifier;
            // Send minecraft:register back with the Fabric channels we support
            // The payload is a list of null-terminated strings
            let mut payload = Vec::new();
            payload.extend_from_slice(b"fabric:registry/sync\0");
            payload.extend_from_slice(b"fabric:registry/sync/complete\0");
            // Register Cardinal Components API channels for mods like Traveler's Backpack
            // This must be done in config phase so the server knows we support CCA
            payload.extend_from_slice(b"cardinal-components:entity_sync\0");
            payload.extend_from_slice(b"cardinal-components:block_sync\0");
            payload.extend_from_slice(b"cardinal-components:chunk_sync\0");
            payload.extend_from_slice(b"cardinal-components:world_sync\0");
            self.ecs
                .commands()
                .trigger(SendConfigPacketEvent::new(
                    self.player,
                    ServerboundCustomPayload {
                        identifier: Identifier::new("minecraft:register"),
                        data: payload.into(),
                    },
                ));
            tracing::info!("Registered Fabric and CCA channels with server");
        }

        // Also emit event for FabricHandshakePlugin to handle c:version/c:register
        as_system::<MessageWriter<_>>(self.ecs, |mut events| {
            events.write(ReceiveConfigPacketEvent {
                entity: self.player,
                packet: std::sync::Arc::new(ClientboundConfigPacket::CustomPayload(p.clone())),
            });
        });
    }

    pub fn disconnect(&mut self, p: &ClientboundDisconnect) {
        warn!("Got disconnect packet {p:?}");
        as_system::<MessageWriter<_>>(self.ecs, |mut events| {
            events.write(DisconnectEvent {
                entity: self.player,
                reason: Some(p.reason.clone()),
            });
        });
    }

    pub fn finish_configuration(&mut self, _p: &ClientboundFinishConfiguration) {
        debug!("got FinishConfiguration packet");

        as_system::<(Commands, Query<&mut RawConnection>)>(
            self.ecs,
            |(mut commands, mut query)| {
                let mut raw_conn = query.get_mut(self.player).unwrap();
                raw_conn.state = ConnectionProtocol::Game;

                commands.trigger(SendConfigPacketEvent::new(
                    self.player,
                    ServerboundFinishConfiguration,
                ));

                // these components are added now that we're going to be in the Game state
                commands
                    .entity(self.player)
                    .remove::<InConfigState>()
                    .insert((
                        crate::JoinedClientBundle::default(),
                        // localentity should already be added, but in case the user forgot or
                        // something we also add it here
                        LocalEntity,
                    ));
            },
        );
    }

    pub fn keep_alive(&mut self, p: &ClientboundKeepAlive) {
        debug!(
            "Got keep alive packet (in configuration) {p:?} for {:?}",
            self.player
        );

        as_system::<(Commands, MessageWriter<_>)>(self.ecs, |(mut commands, mut events)| {
            events.write(KeepAliveEvent {
                entity: self.player,
                id: p.id,
            });
            commands.trigger(SendConfigPacketEvent::new(
                self.player,
                ServerboundKeepAlive { id: p.id },
            ));
        });
    }

    pub fn ping(&mut self, p: &ClientboundPing) {
        debug!("Got ping packet (in configuration) {p:?}");

        as_system::<Commands>(self.ecs, |mut commands| {
            commands.trigger(ConfigPingEvent {
                entity: self.player,
                packet: p.clone(),
            });
        });
    }

    pub fn resource_pack_push(&mut self, p: &ClientboundResourcePackPush) {
        debug!("Got resource pack push packet {p:?}");

        as_system::<MessageWriter<_>>(self.ecs, |mut events| {
            events.write(ResourcePackEvent {
                entity: self.player,
                id: p.id,
                url: p.url.to_owned(),
                hash: p.hash.to_owned(),
                required: p.required,
                prompt: p.prompt.to_owned(),
            });
        });
    }

    pub fn resource_pack_pop(&mut self, p: &ClientboundResourcePackPop) {
        debug!("Got resource pack pop packet {p:?}");
    }

    pub fn update_enabled_features(&mut self, p: &ClientboundUpdateEnabledFeatures) {
        debug!("Got update enabled features packet {p:?}");
    }

    pub fn update_tags(&mut self, _p: &ClientboundUpdateTags) {
        debug!("Got update tags packet");
    }

    pub fn cookie_request(&mut self, p: &ClientboundCookieRequest) {
        debug!("Got cookie request packet {p:?}");
        as_system::<Commands>(self.ecs, |mut commands| {
            commands.trigger(RequestCookieEvent {
                entity: self.player,
                key: p.key.clone(),
            });
        });
    }
    pub fn store_cookie(&mut self, p: &ClientboundStoreCookie) {
        debug!("Got store cookie packet {p:?}");
        as_system::<Commands>(self.ecs, |mut commands| {
            commands.trigger(StoreCookieEvent {
                entity: self.player,
                key: p.key.clone(),
                payload: p.payload.clone(),
            });
        });
    }

    pub fn reset_chat(&mut self, p: &ClientboundResetChat) {
        debug!("Got reset chat packet {p:?}");
    }

    pub fn transfer(&mut self, p: &ClientboundTransfer) {
        debug!("Got transfer packet {p:?}");
    }

    pub fn select_known_packs(&mut self, p: &ClientboundSelectKnownPacks) {
        debug!("Got select known packs packet {p:?}");

        as_system::<Commands>(self.ecs, |mut commands| {
            // resource pack management isn't implemented
            commands.trigger(SendConfigPacketEvent::new(
                self.player,
                ServerboundSelectKnownPacks {
                    known_packs: vec![],
                },
            ));
        });
    }

    pub fn server_links(&mut self, p: &ClientboundServerLinks) {
        debug!("Got server links packet {p:?}");
    }

    pub fn custom_report_details(&mut self, p: &ClientboundCustomReportDetails) {
        debug!("Got custom report details packet {p:?}");
    }

    pub fn clear_dialog(&mut self, p: &ClientboundClearDialog) {
        debug!("Got clear dialog packet {p:?}");
    }
    pub fn show_dialog(&mut self, p: &ClientboundShowDialog) {
        debug!("Got show dialog packet {p:?}");
    }
    pub fn code_of_conduct(&mut self, p: &ClientboundCodeOfConduct) {
        debug!("Got code of conduct packet {p:?}");
    }
}
