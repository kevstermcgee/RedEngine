//! Prints Killchain's weapon tables (docs/WEAPONS.md in the game) from the arsenal, so the document can always be regenerated and never drifts:
//! `cargo run --example weapons_table > projects/killchain/docs/WEAPONS.md`.

use red_engine2::arsenal::Class;
use red_engine2::weapons::Weapon;

fn main() {
    println!("# Killchain weapons\n");
    println!("Generated from `src/arsenal.rs` in RedEngine (`cargo run --example weapons_table`). Damage is per hit to the body; the headshot column is the multiplier.\n");
    println!("| Weapon | Real counterpart | Damage | Head x | Rounds/min | Mag / reserve | Reload s | Auto | Range m |");
    println!("|---|---|---|---|---|---|---|---|---|");
    for w in Weapon::ROSTER
        .iter()
        .filter(|w| w.is_gun() && w.class() != Class::Launcher || matches!(w, Weapon::Lancer | Weapon::Thumper | Weapon::Flare | Weapon::Lobber))
    {
        let k = w.kit();
        let pellets = if k.pellets > 1 { format!(" ({} pellets)", k.pellets) } else { String::new() };
        println!(
            "| {} | {} | {}{} | {} | {:.0} | {} / {} | {} | {} | {:.1} |",
            w.name(),
            k.real,
            k.damage,
            pellets,
            k.head_x10 as f32 / 10.0,
            k.rpm(),
            k.mag,
            k.reserve,
            k.reload,
            k.auto,
            k.range
        );
    }
    println!("\n## Melee");
    println!("| Weapon | Real counterpart | Damage | Swing s | Reach m |");
    println!("|---|---|---|---|---|");
    for w in Weapon::ROSTER.iter().filter(|w| w.is_melee()) {
        let k = w.kit();
        let note = match w {
            Weapon::Knife => " (backstab kills)",
            Weapon::Sledge => " (one hit kills)",
            _ => "",
        };
        println!("| {} | {} | {}{} | {} | {} |", w.name(), k.real, k.damage, note, k.cooldown, k.reach);
    }
    println!("\n## Grenades");
    println!("| Weapon | Real counterpart | Fuse s | Radius m | Blast damage | Lasts s |");
    println!("|---|---|---|---|---|---|");
    for w in Weapon::ROSTER.iter().filter(|w| w.is_grenade()) {
        let k = w.kit();
        let fuse = if k.fuse == 0.0 { "on contact".to_string() } else { k.fuse.to_string() };
        println!(
            "| {} | {} | {} | {} | {} | {} |",
            w.name(),
            k.real,
            fuse,
            k.radius,
            if k.blast > 0 { format!("{}{}", k.blast, if k.lasts > 0.0 { " per tick" } else { "" }) } else { String::new() },
            if k.lasts > 0.0 { k.lasts.to_string() } else { String::new() }
        );
    }
}
