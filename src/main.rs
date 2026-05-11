use gitplot::app::App;

fn main() -> iced::Result {
    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .run_with(|| (App::default(), iced::Task::none()))
}
