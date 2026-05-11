use gitplot::app::App;

fn main() -> iced::Result {
    // tokei (transitively) parallelises parse_from_slice via rayon. On the
    // small per-blob buffers we feed it, the coordination overhead dwarfs
    // the parsing itself — capping rayon to one thread roughly halves wall
    // time. Set before any thread is spawned so rayon picks it up.
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        std::env::set_var("RAYON_NUM_THREADS", "1");
    }

    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| iced::Theme::TokyoNight)
        .window_size(iced::Size::new(1100.0, 720.0))
        .run_with(|| (App::default(), iced::Task::none()))
}
