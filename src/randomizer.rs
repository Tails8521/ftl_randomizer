const MIN_EFFECTIVE_MISSILES: i8 = 28;

#[derive(PartialEq, Eq)]
pub enum WeaponType {
    BEAM,
    BOMB,
    BURST,
    LASER,
    MISSILES,
}

pub struct StartingWeapon {
    name: &'static str,
    weapon_type: WeaponType,
    power: i8,
    sp: i8, // shield piercing
    damage: i8,
    shots: i8, //default to 1 if not present
    ion: i8,
    missiles: i8,
    charge_levels: i8, //default to 1 if not present
}

const ALLOWED_STARTING_WEAPONS_NAMES : [&'static str; 51] = [
    "LASER_BURST_1", // Basic Laser
    "LASER_BURST_2", // Dual Lasers
    "LASER_BURST_2_A", // Burst Laser I
    "LASER_BURST_3", // Burst Laser II
    "LASER_BURST_5", // Burst Laser III
    "LASER_HEAVY_1", // Heavy Laser I
    "LASER_HEAVY_1_SP", // Heavy Pierce I
    "LASER_HEAVY_2", // Heavy Laser II
    "LASER_HULL_1", // Hull Laser I
    "LASER_HULL_2", // Hull Laser II
    "LASER_CHAINGUN", // Chain Laser
    "LASER_CHAINGUN_2", // Chain Vulcan
    "LASER_CHARGEGUN_PLAYER", // Laser Charger+
    "LASER_CHARGEGUN", // Laser Charger
    "LASER_CHARGEGUN_2", // Laser Charger II

    "SHOTGUN_PLAYER", // Adv. Flak
    "SHOTGUN", // Flak I
    "SHOTGUN_2", // Flak II

    "MISSILES_1", // Leto Missiles
    "MISSILES_2_PLAYER", // Artemis Missiles
    "MISSILES_3", // Hermes Missile
    "MISSILES_BREACH", // Breach Missiles
    "MISSILES_HULL", // Hull Missile
    "MISSILE_CHARGEGUN", // Swarm Missiles
    "MISSILES_BURST", // Pegasus Missile

    "BOMB_1", // Small Bomb
    "BOMB_BREACH_1", // Breach Bomb I
    "BOMB_BREACH_2", // Breach Bomb II
    "BOMB_FIRE", // Fire Bomb
    "BOMB_ION", // Ion Bomb
    "BOMB_STUN", // Stun Bomb
    "BOMB_HEAL", // Healing Burst
    "BOMB_HEAL_SYSTEM", // Repair Burst
    "BOMB_LOCK", // Lock Bomb

    "ION_1", // Ion Blast I
    "ION_4", // Ion Blast II
    "ION_2", // Heavy Ion
    "ION_STUN", // Ion Stunner
    "ION_CHARGEGUN", // Ion Charger
    "ION_CHAINGUN", // Chain Ion

    "BEAM_1", // Mini Beam
    "BEAM_LONG", // Pike Beam
    "BEAM_HULL", // Hull Beam
    "BEAM_2", // Halberd Beam
    "BEAM_3", // Glaive Beam
    "BEAM_FIRE", // Fire Beam
    "BEAM_BIO",  // Anti-Bio Beam

    "CRYSTAL_BURST_1", // Crystal Burst I
    "CRYSTAL_BURST_2", // Crystal Burst II
    "CRYSTAL_HEAVY_1", // Crystal Heavy I
    "CRYSTAL_HEAVY_2", // Crystal Heavy II
];

impl StartingWeapon {
    fn qualifies_as_good_enough<'a>(&self, effective_missiles: i8, mut other_weapons: impl Iterator<Item = &'a StartingWeapon>) -> bool {
        if self.weapon_type == WeaponType::BOMB {
            return false;
        }
        if self.weapon_type == WeaponType::BEAM {
            if self.damage == 0 {
                return false;
            } else if self.damage == 1 {
                return other_weapons.any(|w| w.can_break_shield());
            } else {
                return true;
            }
        }
        if self.missiles > 0 {
            if self.damage * self.charge_levels < 2 { // Leto too weak to be viable as only missile
                return false;
            }
            if effective_missiles < MIN_EFFECTIVE_MISSILES {
                return false;
            }
        }
        if self.sp > 1 {
            return true;
        }
        if self.shots * self.charge_levels > 1 {
            return true;
        }
        if self.damage > 0 && other_weapons.any(|w| w.can_break_shield()) {
            return true;
        }
        false
    }

    fn can_break_shield(&self) -> bool {
        match self.weapon_type {
            WeaponType::BEAM => false,
            WeaponType::BOMB => false,
            WeaponType::BURST => {
                if self.missiles > 0 {
                    false
                } else {
                    true
                }
            },
            WeaponType::LASER => true,
            WeaponType::MISSILES => false,
        }
    }
}

fn is_weapon_loadout_viable(starting_weapons: &[StartingWeapon], effective_missiles: i8) -> bool {
    starting_weapons.iter().enumerate().any(|(i, w1)| {
        w1.qualifies_as_good_enough(effective_missiles, starting_weapons.iter().enumerate().filter_map(|(j, w2)| if i != j {Some(w2)} else {None}))
    })
}