use gitplot::app::App;

fn main() -> iced::Result {
    // Tune rayon's global pool. Our git_walk phase-2 already spawns one
    // OS thread per core; tokei then fans out internally on rayon. A
    // default rayon pool (= nproc) plus our nproc workers creates 2×nproc
    // threads on nproc cores → coordination thrash. A small rayon pool
    // (~nproc/4, max 8) is the sweet spot on the test workloads.
    if std::env::var_os("RAYON_NUM_THREADS").is_none() {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2);
        let rayon_threads = (cores / 4).clamp(1, 8);
        std::env::set_var("RAYON_NUM_THREADS", rayon_threads.to_string());
    }

    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(|_| iced::Theme::TokyoNight)
        .window_size(iced::Size::new(1100.0, 720.0))
        .run_with(|| (App::default(), iced::Task::none()))
}
