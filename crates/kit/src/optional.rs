//! Optional programs shared by the shell and companion apps.

pub struct Need {
    pub program: &'static str,
    pub name: &'static str,
    pub store_query: &'static str,
}

pub const GPU_SCREEN_RECORDER: Need = Need {
    program: "gpu-screen-recorder",
    name: "GPU Screen Recorder",
    store_query: "gpu-screen-recorder",
};

pub fn present(need: &Need) -> bool {
    gtk::glib::find_program_in_path(need.program).is_some()
}
