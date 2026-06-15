// Full-game startup smoke test.
//
// Run:
//   cargo test start_complete_game
//
// For interactive scripted testing, use the headless binary instead:
//   cargo run --bin headless -- 'wait 1s; snap /tmp/foo.png'

#[path = "test_lib.rs"]
mod test_lib;
use test_lib::{headless_game_app, tick};

#[test]
fn start_complete_game() {
    let mut app = headless_game_app(Some(42));

    // Let startup systems run (OnEnter(Restart) → spawn world → transition to InLevel).
    for _ in 0..120 {
        tick(&mut app);
    }

    let state = app.world().resource::<State<vector_arena::GameState>>();
    assert_eq!(
        *state.get(),
        vector_arena::GameState::InLevel,
        "expected GameState::InLevel after startup"
    );
    let player_count =
        app.world_mut().query::<&vector_arena::player::Player>().iter(app.world()).count();
    assert_eq!(player_count, 1, "expected exactly one Player entity after startup");
}
