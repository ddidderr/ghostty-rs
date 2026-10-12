//! The C types of `include/ghostty.h`, laid out exactly as the header (and
//! the Zig `extern` structs) define them.
//!
//! Enums are `c_int` everywhere (the header uses `int`-sized enums and the C
//! caller may pass any value); the enumerator values live in submodules as
//! `i32` constants named after the header's enumerators.
#![allow(non_camel_case_types, clippy::struct_field_names)]

use std::ffi::{c_char, c_int, c_void};

// ---------------------------------------------------------------------
// Opaque handles

pub type ghostty_app_t = *mut c_void;
pub type ghostty_config_t = *mut c_void;
pub type ghostty_surface_t = *mut c_void;
pub type ghostty_inspector_t = *mut c_void;

// ---------------------------------------------------------------------
// Scalars and enums (all `int`)

pub type ghostty_platform_e = c_int;
pub type ghostty_clipboard_e = c_int;
pub type ghostty_clipboard_request_e = c_int;
pub type ghostty_input_mouse_state_e = c_int;
pub type ghostty_input_mouse_button_e = c_int;
pub type ghostty_input_mouse_momentum_e = c_int;
pub type ghostty_color_scheme_e = c_int;
pub type ghostty_input_scroll_mods_t = c_int;
pub type ghostty_input_mods_e = c_int;
pub type ghostty_binding_flags_e = c_int;
pub type ghostty_input_action_e = c_int;
pub type ghostty_input_key_e = c_int;
pub type ghostty_input_trigger_tag_e = c_int;
pub type ghostty_build_mode_e = c_int;
pub type ghostty_point_tag_e = c_int;
pub type ghostty_point_coord_e = c_int;
pub type ghostty_surface_context_e = c_int;
pub type ghostty_target_tag_e = c_int;
pub type ghostty_action_tag_e = c_int;

/// `GHOSTTY_SUCCESS`.
pub const GHOSTTY_SUCCESS: c_int = 0;

pub mod platform {
    pub const INVALID: i32 = 0;
    pub const MACOS: i32 = 1;
    pub const IOS: i32 = 2;
}

pub mod clipboard {
    pub const STANDARD: i32 = 0;
    pub const SELECTION: i32 = 1;
}

pub mod trigger_tag {
    pub const PHYSICAL: i32 = 0;
    pub const UNICODE: i32 = 1;
    pub const CATCH_ALL: i32 = 2;
}

pub mod build_mode {
    pub const DEBUG: i32 = 0;
    pub const RELEASE_SAFE: i32 = 1;
    pub const RELEASE_FAST: i32 = 2;
    pub const RELEASE_SMALL: i32 = 3;
}

pub mod target_tag {
    pub const APP: i32 = 0;
    pub const SURFACE: i32 = 1;
}

// ---------------------------------------------------------------------
// Structs

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_clipboard_content_s {
    pub mime: *const c_char,
    pub data: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_input_key_s {
    pub action: ghostty_input_action_e,
    pub mods: ghostty_input_mods_e,
    pub consumed_mods: ghostty_input_mods_e,
    pub keycode: u32,
    pub text: *const c_char,
    pub unshifted_codepoint: u32,
    pub composing: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_input_trigger_key_u {
    pub translated: ghostty_input_key_e,
    pub physical: ghostty_input_key_e,
    pub unicode: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_input_trigger_s {
    pub tag: ghostty_input_trigger_tag_e,
    pub key: ghostty_input_trigger_key_u,
    pub mods: ghostty_input_mods_e,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_command_s {
    pub action_key: *const c_char,
    pub action: *const c_char,
    pub title: *const c_char,
    pub description: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_info_s {
    pub build_mode: ghostty_build_mode_e,
    pub version: *const c_char,
    pub version_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_diagnostic_s {
    pub message: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_string_s {
    pub ptr: *const c_char,
    pub len: usize,
    pub sentinel: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_text_s {
    pub tl_px_x: f64,
    pub tl_px_y: f64,
    pub offset_start: u32,
    pub offset_len: u32,
    pub text: *const c_char,
    pub text_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_point_s {
    pub tag: ghostty_point_tag_e,
    pub coord: ghostty_point_coord_e,
    pub x: u32,
    pub y: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_selection_s {
    pub top_left: ghostty_point_s,
    pub bottom_right: ghostty_point_s,
    pub rectangle: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_env_var_s {
    pub key: *const c_char,
    pub value: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_platform_macos_s {
    pub nsview: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_platform_ios_s {
    pub uiview: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_platform_u {
    pub macos: ghostty_platform_macos_s,
    pub ios: ghostty_platform_ios_s,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_surface_config_s {
    pub platform_tag: ghostty_platform_e,
    pub platform: ghostty_platform_u,
    pub userdata: *mut c_void,
    pub scale_factor: f64,
    pub font_size: f32,
    pub working_directory: *const c_char,
    pub command: *const c_char,
    pub env_vars: *mut ghostty_env_var_s,
    pub env_var_count: usize,
    pub initial_input: *const c_char,
    pub wait_after_command: bool,
    pub context: ghostty_surface_context_e,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_surface_size_s {
    pub columns: u16,
    pub rows: u16,
    pub width_px: u32,
    pub height_px: u32,
    pub cell_width_px: u32,
    pub cell_height_px: u32,
}

// ---- config types

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_path_s {
    pub path: *const c_char,
    pub optional: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_color_s {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_color_list_s {
    pub colors: *const ghostty_config_color_s,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_command_list_s {
    pub commands: *const ghostty_command_s,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_palette_s {
    pub colors: [ghostty_config_color_s; 256],
}

pub mod quick_terminal_size_tag {
    pub const NONE: i32 = 0;
    pub const PERCENTAGE: i32 = 1;
    pub const PIXELS: i32 = 2;
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_quick_terminal_size_value_u {
    pub percentage: f32,
    pub pixels: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_quick_terminal_size_s {
    pub tag: c_int,
    pub value: ghostty_quick_terminal_size_value_u,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_config_quick_terminal_size_s {
    pub primary: ghostty_quick_terminal_size_s,
    pub secondary: ghostty_quick_terminal_size_s,
}

// ---- targets and actions

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_target_u {
    pub surface: ghostty_surface_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_target_s {
    pub tag: ghostty_target_tag_e,
    pub target: ghostty_target_u,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_resize_split_s {
    pub amount: u16,
    pub direction: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_move_tab_s {
    pub amount: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_desktop_notification_s {
    pub title: *const c_char,
    pub body: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_set_title_s {
    pub title: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_pwd_s {
    pub pwd: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_mouse_over_link_s {
    pub url: *const c_char,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_size_limit_s {
    pub min_width: u32,
    pub min_height: u32,
    pub max_width: u32,
    pub max_height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_initial_size_s {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_cell_size_s {
    pub width: u32,
    pub height: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_key_sequence_s {
    pub active: bool,
    pub trigger: ghostty_input_trigger_s,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_key_table_activate_s {
    pub name: *const c_char,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_action_key_table_u {
    pub activate: ghostty_action_key_table_activate_s,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_key_table_s {
    pub tag: c_int,
    pub value: ghostty_action_key_table_u,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_color_change_s {
    pub kind: c_int,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_config_change_s {
    pub config: ghostty_config_t,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_reload_config_s {
    pub soft: bool,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_open_url_s {
    pub kind: c_int,
    pub url: *const c_char,
    pub len: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_surface_message_childexited_s {
    pub exit_code: u32,
    pub timetime_ms: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_progress_report_s {
    pub state: c_int,
    pub progress: i8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_command_finished_s {
    pub exit_code: i16,
    pub duration: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_start_search_s {
    pub needle: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_search_total_s {
    pub total: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_search_selected_s {
    pub selected: isize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_scrollbar_s {
    pub total: u64,
    pub offset: u64,
    pub len: u64,
}

/// `ghostty_action_u`. Fields that are plain enums are `c_int`.
#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_action_u {
    pub new_split: c_int,
    pub toggle_fullscreen: c_int,
    pub move_tab: ghostty_action_move_tab_s,
    pub goto_tab: c_int,
    pub goto_split: c_int,
    pub goto_window: c_int,
    pub resize_split: ghostty_action_resize_split_s,
    pub size_limit: ghostty_action_size_limit_s,
    pub initial_size: ghostty_action_initial_size_s,
    pub cell_size: ghostty_action_cell_size_s,
    pub scrollbar: ghostty_action_scrollbar_s,
    pub inspector: c_int,
    pub desktop_notification: ghostty_action_desktop_notification_s,
    pub set_title: ghostty_action_set_title_s,
    pub set_tab_title: ghostty_action_set_title_s,
    pub prompt_title: c_int,
    pub pwd: ghostty_action_pwd_s,
    pub mouse_shape: c_int,
    pub mouse_visibility: c_int,
    pub mouse_over_link: ghostty_action_mouse_over_link_s,
    pub renderer_health: c_int,
    pub quit_timer: c_int,
    pub float_window: c_int,
    pub secure_input: c_int,
    pub key_sequence: ghostty_action_key_sequence_s,
    pub key_table: ghostty_action_key_table_s,
    pub color_change: ghostty_action_color_change_s,
    pub reload_config: ghostty_action_reload_config_s,
    pub config_change: ghostty_action_config_change_s,
    pub open_url: ghostty_action_open_url_s,
    pub close_tab_mode: c_int,
    pub child_exited: ghostty_surface_message_childexited_s,
    pub progress_report: ghostty_action_progress_report_s,
    pub command_finished: ghostty_action_command_finished_s,
    pub start_search: ghostty_action_start_search_s,
    pub search_total: ghostty_action_search_total_s,
    pub search_selected: ghostty_action_search_selected_s,
    pub readonly: c_int,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_action_s {
    pub tag: ghostty_action_tag_e,
    pub action: ghostty_action_u,
}

// ---- runtime callbacks

pub type ghostty_runtime_wakeup_cb = Option<unsafe extern "C" fn(*mut c_void)>;
pub type ghostty_runtime_read_clipboard_cb =
    Option<unsafe extern "C" fn(*mut c_void, ghostty_clipboard_e, *mut c_void) -> bool>;
pub type ghostty_runtime_confirm_read_clipboard_cb = Option<
    unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void, ghostty_clipboard_request_e),
>;
pub type ghostty_runtime_write_clipboard_cb = Option<
    unsafe extern "C" fn(
        *mut c_void,
        ghostty_clipboard_e,
        *const ghostty_clipboard_content_s,
        usize,
        bool,
    ),
>;
pub type ghostty_runtime_close_surface_cb = Option<unsafe extern "C" fn(*mut c_void, bool)>;
pub type ghostty_runtime_action_cb =
    Option<unsafe extern "C" fn(ghostty_app_t, ghostty_target_s, ghostty_action_s) -> bool>;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_runtime_config_s {
    pub userdata: *mut c_void,
    pub supports_selection_clipboard: bool,
    pub wakeup_cb: ghostty_runtime_wakeup_cb,
    pub action_cb: ghostty_runtime_action_cb,
    pub read_clipboard_cb: ghostty_runtime_read_clipboard_cb,
    pub confirm_read_clipboard_cb: ghostty_runtime_confirm_read_clipboard_cb,
    pub write_clipboard_cb: ghostty_runtime_write_clipboard_cb,
    pub close_surface_cb: ghostty_runtime_close_surface_cb,
}

// ---- ipc (declared by the header; the embedded runtime never uses them)

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_ipc_target_u {
    pub klass: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct chostty_ipc_target_s {
    pub tag: c_int,
    pub target: ghostty_ipc_target_u,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ghostty_ipc_action_new_window_s {
    pub arguments: *mut *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union ghostty_ipc_action_u {
    pub new_window: ghostty_ipc_action_new_window_s,
}
