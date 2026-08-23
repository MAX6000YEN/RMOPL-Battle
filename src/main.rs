use macroquad::prelude::*;

const BACKGROUND: Color = Color::new(0.07, 0.07, 0.09, 1.0);

#[macroquad::main("RMOPL Battle")]
async fn main() {
    while !is_key_pressed(KeyCode::Escape) {
        clear_background(BACKGROUND);
        next_frame().await;
    }
}
