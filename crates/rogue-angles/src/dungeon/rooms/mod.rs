//! Stock `RoomKind` implementations, opt-in via `RoomRegistry::stock()`. Each room here
//! uses only the public `LevelBuilder` API from `level_generation` — no shortcuts — since
//! keeping every stock room in the engine means they no longer pressure-test the
//! `RoomKind` trait by being ported out to a game; a custom `RoomKind` in a downstream
//! example game is the other half of that check.
mod colonnade;
mod normal;
mod octagon;
mod oval;
mod ring;
mod rubble;
mod slow_zone;
mod vault;

pub use colonnade::ColonnadeRoom;
pub use normal::NormalRoom;
pub use octagon::OctagonRoom;
pub use oval::OvalRoom;
pub use ring::RingRoom;
pub use rubble::RubbleRoom;
pub use slow_zone::SlowZoneRoom;
pub use vault::VaultRoom;

use crate::dungeon::level_generation::RoomRegistry;

impl RoomRegistry {
    /// The engine's built-in room library, with weights chosen to approximate the
    /// proportions this game shipped with before rooms became pluggable: ~92% Normal,
    /// ~1.33% each of the six other specials. Oval is registered but weighted at 0 (never
    /// drawn) pending investigation of a performance issue with its polygon — a game can
    /// still select it explicitly if it wants.
    pub fn stock() -> Self {
        let mut registry = Self::new();
        registry
            .register(NormalRoom)
            .register(OvalRoom)
            .register(ColonnadeRoom)
            .register(SlowZoneRoom)
            .register(OctagonRoom)
            .register(VaultRoom)
            .register(RubbleRoom)
            .register(RingRoom);
        registry
    }
}
