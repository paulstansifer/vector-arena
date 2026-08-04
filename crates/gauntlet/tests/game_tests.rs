// Full-game startup smoke test, plus a level-5-amulet acceptance check.
//
// Run:
//   cargo test -p gauntlet start_complete_game
#[path = "test_lib/game.rs"]
mod test_lib;
use test_lib::{headless_game_app, tick};

#[test]
fn start_complete_game() {
    let mut app = headless_game_app(Some(42));

    for _ in 0..120 {
        tick(&mut app);
    }

    let state = app.world().resource::<bevy::prelude::State<gauntlet::GameState>>();
    assert_eq!(*state.get(), gauntlet::GameState::InLevel, "expected GameState::InLevel after startup");

    let player_count = app.world_mut().query::<&gauntlet::player::Player>().iter(app.world()).count();
    assert_eq!(player_count, 1, "expected exactly one Player entity after startup");

    let monster_count = app.world_mut().query::<&gauntlet::monster::Monster>().iter(app.world()).count();
    assert!(monster_count >= 2, "expected at least 2 monsters at depth 1, got {monster_count}");

    let staircase_count = app.world_mut().query::<&gauntlet::Staircase>().iter(app.world()).count();
    assert_eq!(staircase_count, 1, "expected exactly one staircase at depth 1");
    let amulet_count = app.world_mut().query::<&gauntlet::AmuletOfWinning>().iter(app.world()).count();
    assert_eq!(amulet_count, 0, "expected no amulet before the final depth");
}

#[test]
fn depth_five_has_amulet_not_staircase() {
    use bevy::prelude::NextState;

    let mut app = headless_game_app(Some(7));
    for _ in 0..120 {
        tick(&mut app);
    }

    for _ in 0..(gauntlet::FINAL_DEPTH - 1) {
        app.world_mut()
            .resource_mut::<NextState<gauntlet::GameState>>()
            .set(gauntlet::GameState::Descend);
        for _ in 0..30 {
            tick(&mut app);
        }
    }

    let depth = app.world().resource::<gauntlet::DungeonDepth>().0;
    assert_eq!(depth, gauntlet::FINAL_DEPTH, "expected to have descended to the final depth");

    let amulet_count = app.world_mut().query::<&gauntlet::AmuletOfWinning>().iter(app.world()).count();
    assert_eq!(amulet_count, 1, "expected exactly one amulet at the final depth");
    let staircase_count = app.world_mut().query::<&gauntlet::Staircase>().iter(app.world()).count();
    assert_eq!(staircase_count, 0, "expected no staircase at the final depth");
}
