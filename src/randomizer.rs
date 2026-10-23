// AI agents are not allowed to edit this file, unless it's something trivial like struct/field visibility or derive.
// This file contains randomizer logic only, keeping the xml parsing etc. out of it.

use crate::rng::Rng;

pub const MIN_EFFECTIVE_MISSILES: i8 = 28;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WeaponType {
    BEAM,
    BOMB,
    BURST,
    LASER,
    MISSILES,
}

#[derive(Clone, Copy)]
pub struct StartingWeapon {
    pub name: &'static str,
    pub weapon_type: WeaponType,
    pub power: i8,
    pub sp: i8, // shield piercing
    pub damage: i8,
    pub shots: i8, //default to 1 if not present
    pub ion: i8,
    pub missiles: i8,
    pub charge_levels: i8, //default to 1 if not present
}

pub const ALLOWED_STARTING_WEAPONS_NAMES : [&'static str; 51] = [
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
        if self.weapon_type == WeaponType::BURST { // Flak weapons
            return true;
        }
        if self.sp >= 1 {
            return true;
        }
        if self.shots * self.charge_levels > 1 && self.damage > 0 {
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

pub fn is_weapon_loadout_viable(starting_weapons: &[StartingWeapon], effective_missiles: i8) -> bool {
    starting_weapons.iter().enumerate().any(|(i, w1)| {
        w1.qualifies_as_good_enough(effective_missiles, starting_weapons.iter().enumerate().filter_map(|(j, w2)| if i != j {Some(w2)} else {None}))
    })
}

#[derive(Clone, Copy)]
pub struct StartingDrone{
    pub name: &'static str,
    pub power: i8,
}

pub const ALLOWED_STARTING_DRONES_NAMES : [&'static str; 15] = [
    "BOARDER",
    "BOARDER_ION",

    "COMBAT_1",
    "COMBAT_2",
    "COMBAT_BEAM",
    "COMBAT_BEAM_2",
    "COMBAT_FIRE",

    "BATTLE",
    "REPAIR",

    "DEFENSE_1",
    "DEFENSE_2",
    "ANTI_DRONE",
    "DRONE_SHIELD",
    "DRONE_SHIELD_PLAYER",
    "SHIP_REPAIR",
];

impl StartingDrone {
    fn qualifies_as_good_enough<'a>(&self, mut starting_weapons: impl Iterator<Item = &'a StartingWeapon>) -> bool {
        match self.name {
            "COMBAT_1" | "COMBAT_2" | "COMBAT_BEAM" | "COMBAT_BEAM_2" => starting_weapons.any(|w| w.ion > 0),
            _ => false,
        }
    }
}

pub fn is_drone_loadout_viable(starting_drones: &[StartingDrone], starting_weapons: &[StartingWeapon]) -> bool {
    starting_drones.iter().any(|d| d.qualifies_as_good_enough(starting_weapons.iter()))
}

pub struct Augment {
    pub name: String,
    pub stackable: bool, // if we roll the same augment twice, reroll if that augment isn't stackable
}

pub const STARTING_CREW : [&'static str; 22] = [
    "human", "human", "human",
    "engi", "engi", "engi",
    "mantis", "mantis", "mantis",
    "rock", "rock", "rock",
    "energy", "energy", "energy", // zoltan
    "slug", "slug", "slug",
    "crystal", // 3 times rarer than the rest
    "anaerobic", "anaerobic", "anaerobic", // lanius
];

fn is_crew_tp_viable(starting_crew: &[&str]) -> bool {
    let mut mid = 0;
    let mut strong = 0;
    for &crew in starting_crew {
        match crew {
            "human" | "slug" |"lanius" => mid += 1,
            "rock" | "crystal" |"mantis" => strong += 1,
            _ => {}
        }
    }
    strong >= 2 || strong + mid >= 2 && starting_crew.len() >= 3
}

pub struct RandomizedShip {
    pub augments: Vec<String>, // up to 3 augments (0: 14%, 1: 75%, 2: 10%, 3: 1%)
    pub weapons: Vec<StartingWeapon>, // up to 3 or 4 weapons, depending of the amount of weapon slots.
    pub drones: Vec<StartingDrone>, // up to 2 or 3 drones, depending of the amount of drone slots.
    pub crew: Vec<&'static str>, // 1-4 crewmates
    pub systems: Vec<(&'static str, usize)>,
    pub reactor: usize,
    pub artillery_weapon: &'static str, // only used on ships that have artillery system (fed)
}

pub struct Randomizer {
    pub starting_augments_pool: Vec<Augment>,

    // same order as starting_augments_pool, based on the augment's rarity:
    // rarity 1 -> weight 12, 2 -> 6, 3 -> 4, 4 -> 3, 0 -> 3
    pub augment_weights: Vec<u32>,
    pub starting_weapons_pool: Vec<StartingWeapon>,
    pub starting_drones_pool: Vec<StartingDrone>,
}

impl Randomizer {
    pub fn randomize_ship(&self, rng: &mut Rng, has_4_weapons_slots: bool, has_3_drones_slots: bool, has_artillery: bool) -> RandomizedShip {
        assert_eq!(self.starting_weapons_pool.len(), ALLOWED_STARTING_WEAPONS_NAMES.len());
        assert_eq!(self.starting_drones_pool.len(), ALLOWED_STARTING_DRONES_NAMES.len());
        loop {
            let mut augments = Vec::new();
            let mut weapons = Vec::new();
            let mut drones = Vec::new();
            let mut crew = Vec::new();
            let mut systems = Vec::new();
            let mut reactor = 0;
            let mut artillery_weapon = "";

            let crew_count = rng.pick(&[0, 1, 4, 8, 3]);
            if rng.below(2) == 0 {
                // all random
                for _ in 0..crew_count {
                    crew.push(STARTING_CREW[rng.index(STARTING_CREW.len())]);
                }
            } else {
                // biased towards a specific race
                let first = STARTING_CREW[rng.index(STARTING_CREW.len())];
                crew.push(first);
                for _ in 0..crew_count-1 {
                    if rng.below(2) == 0 {
                        crew.push(first);
                    } else {
                        crew.push(STARTING_CREW[rng.index(STARTING_CREW.len())]);
                    }
                }
            }

            let mut total_weapon_power = rng.pick(&[1, 1, 4, 4, 2]);
            let mut weapon_power_available = total_weapon_power;
            let mut weapon_slots_available = if has_4_weapons_slots { 4 } else { 3 };
            while weapon_power_available > 0 && weapon_slots_available > 0 {
                loop {
                    let to_add = self.starting_weapons_pool[rng.index(self.starting_weapons_pool.len())];
                    if to_add.power as usize <= weapon_power_available {
                        weapons.push(to_add);
                        weapon_power_available -= to_add.power as usize;
                        break;
                    }
                }
                weapon_slots_available -= 1;
            }
            reactor += total_weapon_power;
            if total_weapon_power == 0 {
                total_weapon_power = 1;
            } else if total_weapon_power < 4 && weapon_slots_available > 0 && rng.below(6) == 0 {
                // Some ships get an extra 1 power weapon that they may be able to power on later
                loop {
                    let to_add = self.starting_weapons_pool[rng.index(self.starting_weapons_pool.len())];
                    if to_add.power == 1 {
                        weapons.push(to_add);
                        break;
                    }
                }
            }
            systems.push(("weapons", total_weapon_power));

            let augments_to_add = rng.pick(&[14, 75, 10, 1]);
            for _ in 0..augments_to_add {
                loop {
                    let to_add = &self.starting_augments_pool[rng.pick(&self.augment_weights)];
                    if to_add.stackable || !augments.iter().any(|a| *a == to_add.name) {
                        augments.push(String::from(&to_add.name));
                        break;
                    }
                }
            }

            systems.push(("oxygen", 1)); // TODO check if the game allows you to buy oxygen at stores
            reactor += 1;
            
            let engines_level = rng.pick(&[0, 10, 10, 1, 1]);
            systems.push(("engines", engines_level));
            reactor += engines_level;
            
            
            let mut additional_systems_to_add = rng.pick(&[50, 50, 20, 1]);
            if has_artillery {
                additional_systems_to_add = additional_systems_to_add.saturating_sub(1);
                let artillery_power = rng.pick(&[0, 2, 1]);
                reactor += artillery_power;
                artillery_weapon = if rng.below(2) == 0 {
                    "ARTILLERY_FED"
                } else {
                    "ARTILLERY_FED_C"
                }
            }
            for _ in 0..additional_systems_to_add {
                loop {
                    const ADDITIONAL_SYSTEMS: [&'static str; 5] = ["teleporter", "cloaking", "drones", "hacking", "mind"];
                    let to_add = ADDITIONAL_SYSTEMS[rng.index(ADDITIONAL_SYSTEMS.len())];
                    if !systems.iter().any(|(s, l)| *s == to_add && *l > 0) {
                        systems.push((to_add, 1));
                        reactor += 1;
                        break;
                    }
                }
            }
            
            let shield_weights = if augments.iter().any(|a| *a == "ENERGY_SHIELD" || *a == "ADV_SCANNERS" && systems.iter().any(|(s, l)| *s == "cloaking" && *l > 0)) {
                [1, 1, 2, 0, 0]
            } else {
                [0, 0, 29, 0, 1]
            };
            let shields_level = rng.pick(&shield_weights);
            if shields_level > 0 {
                systems.push(("shields", shields_level));
                reactor += shields_level;
            }

            if rng.below(2) == 0 { // if no heal system, then we'll check later if heal burst and enough missiles
                if rng.below(2) == 0 {
                    systems.push(("medbay", rng.pick(&[0, 200, 10, 1])));
                } else {
                    systems.push(("clonebay", rng.pick(&[0, 200, 10, 1])));
                }
            }

            systems.push(("pilot", rng.pick(&[0, 200, 10, 1])));
            systems.push(("doors", rng.pick(&[10, 200, 10, 1])));
            systems.push(("sensors", rng.pick(&[40, 200, 10, 1])));
            if rng.below(21) == 0 {
                systems.push(("battery", rng.pick(&[0, 9, 1])));
                reactor -= 1;
            }

            if systems.iter().any(|(s, l)| *s == "drones" && *l > 0) {
                let total_drone_power = rng.pick(&[0, 0, 4, 4, 2]);
                let mut drone_power_available = total_drone_power;
                let mut drone_slots_available = if has_3_drones_slots { 3 } else { 2 };
                while drone_power_available > 0 && drone_slots_available > 0 {
                    loop {
                        let to_add = self.starting_drones_pool[rng.index(self.starting_drones_pool.len())];
                        if to_add.power as usize <= drone_power_available {
                            drones.push(to_add);
                            drone_power_available -= to_add.power as usize;
                            break;
                        }
                    }
                    drone_slots_available -= 1;
                }
                reactor += total_drone_power;
                if total_drone_power < 4 && drone_slots_available > 0 && rng.below(6) == 0 {
                    // Some ships get an extra 1-2 power drone that they may be able to power on
                    loop {
                        let to_add = self.starting_drones_pool[rng.index(self.starting_drones_pool.len())];
                        if to_add.power <= 2 {
                            drones.push(to_add);
                            break;
                        }
                    }
                }
            }
            
            let missiles = weapons.iter().fold(0, |acc, w| acc + (w.missiles * 10)) + rng.below(31) as i8;
            let effective_missiles = if augments.iter().any(|a| a == "EXPLOSIVE_REPLICATOR") {
                missiles.saturating_mul(2)
            } else {
                missiles
            };
            if is_weapon_loadout_viable(&weapons, effective_missiles)
            || is_drone_loadout_viable(&drones, &weapons)
            || systems.iter().any(|(s, l)| *s == "teleporter" && *l > 0) && is_crew_tp_viable(&crew) {
                systems.retain(|(_, l)| *l > 0); // Remove level 0 systems
                return RandomizedShip {
                    augments,
                    weapons,
                    drones,
                    crew,
                    systems,
                    reactor,
                    artillery_weapon,
                };
            }
        }
    }
}