pub mod auth_user;
pub mod default_selection;
pub mod environment;
pub mod issue_helpers;
pub mod session;
pub mod settings;
pub mod user;
pub mod num_lock;

use std::env;
use std::process::Command;
use ncursesw::*;
use ncursesw::normal::{Attributes, ColorPair, Colors};
use gettextrs::{setlocale, LocaleCategory};

use crate::auth_user::auth_user;
use crate::issue_helpers::draw_on_screen;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize ncurses
    setlocale(LocaleCategory::LcAll, "");

    initscr()?;
    keypad(stdscr(), true)?;
    noecho()?;
    start_color()?;

    let tty_path = environment::get_tty_path();

    // Set the config path to default or to arg1 if provided
    let default_path = String::from("/etc/nclogin/config.toml");
    let args: Vec<String> = env::args().collect();
    let config_path = args.get(1).unwrap_or(&default_path);

    // Parse the settings TOML file into the config struct if the file exists
    // Otherwise try to create the file using default config
    // If fails use default
    let config = settings::parse_settings(config_path);

    // Read the issue file if the file exists
    // Otherwise write default to the file
    // If fails use default
    let issue_lines = issue_helpers::read_or_generate_issue_file(&config.issue_file_settings.issue_file);

    // Read Sessions from sessions TOML file if the file exists
    // Otherwise try to parse sessions from shell file, x11 dir and wayland dir and write them to toml file
    let sessions = session::get_sessions(
        &config.login_behaviour.session_file,
        &config.login_behaviour.shells_file,
        &config.login_behaviour.x11_session_folder,
        &config.login_behaviour.wayland_session_folder)?;

    // Create color pairs
    let colors_normal = Colors::new(
        settings::to_color(&config.colors.normal_fg_color),
        settings::to_color(&config.colors.normal_bg_color));
    let colors_highlight = Colors::new(
        settings::to_color(&config.colors.highlight_fg_color),
        settings::to_color(&config.colors.highlight_bg_color));
    let color_pair_normal = ColorPair::new(1,colors_normal)?;
    let color_pair_highlight = ColorPair::new(2, colors_highlight)?;
    let attrs = Attributes::default();
    attr_set(attrs, color_pair_normal)?;

    // Num lock on startup
    if config.login_behaviour.activate_num_lock {
        if let Err(e) = num_lock::set_num_lock_tty(true) {
            eprintln!("Failed to set Num Lock on TTY: {}", e);
        }
    }


    // Parse users
    let users = user::parse_valid_users(
        &config.login_behaviour.user_file,
        &config.login_behaviour.shells_file,
        *&config.login_behaviour.min_uid,
        *&config.login_behaviour.include_root_user
    )?;

    let mut selected_user :usize;
    let mut selected_session :usize;

    (selected_user, selected_session) = match default_selection::get_default_indices(
        &config.login_behaviour.default_selection_file, &users, &sessions) {
        Ok((user_index, session_index)) => (user_index, session_index),
        Err(e) => {
            eprintln!("Error reading default indices: {}", e);
            (0, 0) // Default to 0, 0 if there's an error
        }
    };

    draw_on_screen(
        issue_lines,
        config.issue_file_settings.issue_col_gap,
        config.issue_file_settings.issue_row_gap);
    refresh()?;

    loop {

        let size = getmaxyx(stdscr())?;
        curs_set(CursorType::Invisible)?;

        // Display top bar
        // TODO
        // Show Command
        // Show Separator

        // Display bottom bar
        // TODO
        // Show Command
        // Show Separator

        let mut position = Origin{y: config.user_prompt.user_option_row_gap as i32, x: config.user_prompt.user_option_col_gap as i32 };
        let status_bar_pos = Origin{y: size.lines-1, x: 0};
        clear_line(status_bar_pos)?;

        // Display user selection prompt
        mvaddstr(position, &config.user_prompt.user_option_prompt)?; // USER_OPTION_PROMPT
        position.y += 1;
        position.x += (&config.user_prompt.user_option_prompt.len() / 2) as i32;
        mvaddstr(position, "↑")?;
        position.y += 1;
        position.x = config.user_prompt.user_option_col_gap as i32;

        for (i, user) in users.iter().enumerate() {
            if i == selected_user {
                attr_set(attrs, color_pair_highlight)?;
                mvaddstr(position, &user.name)?;
                attr_set(attrs, color_pair_normal)?;
            }
            else {
                mvaddstr(position, &user.name)?;
            }
            position.y +=1;
        }
        position.x += (&config.user_prompt.user_option_prompt.len() / 2) as i32;
        mvaddstr(position, "↑")?;

        mvaddstr(position, "↓")?;

        position.y += config.start_prompt.start_option_row_gap as i32;
        position.x = config.start_prompt.start_option_col_gap as i32;

        mvaddstr(position, &config.start_prompt.start_option_prompt)?; // START_OPTION_PROMPT

        position.y += 1;
        clear_line(position)?;
        mvaddstr(position, "←")?;
        attr_set(attrs, color_pair_highlight)?;
        position.x +=2;
        mvaddstr(position, &sessions[selected_session].name)?;
        attr_set(attrs, color_pair_normal)?;
        position.x += sessions[selected_session].name.len() as i32 + 1;
        mvaddstr(position, "→")?;

        // Handle keyboard input
        let ch = getch()?;
        match ch
        {
            CharacterResult::Character('k') | CharacterResult::Key(KeyBinding::UpArrow) => {
                if selected_user > 0 {
                    selected_user -= 1;
                }
            }
            CharacterResult::Character('j') | CharacterResult::Key(KeyBinding::DownArrow) => {
                if selected_user < users.len() - 1 {
                    selected_user += 1;
                }
            }
            CharacterResult::Character('h') | CharacterResult::Key(KeyBinding::LeftArrow) => {
                if selected_session == 0 {
                    selected_session = sessions.len() - 1;
                } else {
                    selected_session -= 1;
                }
            }
            CharacterResult::Character('l') | CharacterResult::Key(KeyBinding::RightArrow) => {
                selected_session = (selected_session + 1) % sessions.len();
            }
            CharacterResult::Key(KeyBinding::FunctionKey(1)) => {
                mvaddstr(Origin { y: 0, x: 0 }, "reboot")?;
                if issue_helpers::get_logged_in_users() < 1 {
                    Command::new("reboot").status().unwrap();
                } else {
                    mvaddstr(status_bar_pos, "→ reboot not possible, users are logged in")?;
                }
            }
            CharacterResult::Key(KeyBinding::FunctionKey(2)) => {
                mvaddstr(Origin { y: 0, x: 0 }, "shutdown")?;
                if issue_helpers::get_logged_in_users() < 1 {
                    Command::new("shutdown").arg("--poweroff").status().unwrap();
                } else {
                    mvaddstr(status_bar_pos, "→ shutdown not possible, users are logged in")?;
                }
            }
            CharacterResult::Key(KeyBinding::Enter) | CharacterResult::Character('\n') => {
                // Print command that is executed
                let command = &sessions[selected_session].cmd;
                let cmd_dsp_str = format!("→ {}", command);
                clear_line(status_bar_pos)?;
                mvaddstr(status_bar_pos, cmd_dsp_str)?;
                position.y += config.password_prompt.password_row_gap as i32;
                position.x = config.password_prompt.password_col_gap as i32;

                mvaddstr(position, &config.password_prompt.password_prompt)?; // PASSWORD_PROMPT

                curs_set(CursorType::Visible)?;

                // Use getnstr to capture the password
                let password = getnstr(2000)?;
                curs_set(CursorType::Invisible)?;
                position.x = config.password_prompt.password_col_gap as i32;
                position.y = position.y + 2;

                wmove(stdscr(), position)?;

                if auth_user(&users[selected_user].name, &password, &tty_path) {
                    // Write default selection if activated
                    if config.login_behaviour.write_last_to_default_selection {
                        match default_selection::write_selection(&config.login_behaviour.default_selection_file, &users[selected_user], &sessions[selected_session]) {
                            Err(e) => eprintln!("Error writing default File: {}", e),
                            _ => {}
                        }
                    }
                    clear()?;
                    refresh()?;
                    endwin()?;
                    environment::exec_session_as_user(&users[selected_user], &sessions[selected_session]);
                    return Ok(());
                } else {
                    mvaddstr(position, "Authentication failed. Press enter to try again...")?;
                    position.y -= 2;
                }
            }
            // If any other key do nothing
            _ => {}
        }
    }
}

fn clear_line(p0: Origin) -> Result<(), Box<dyn std::error::Error>> {
    wmove(stdscr(), p0)?; // Move cursor to the beginning of the line
    wclrtoeol(stdscr())?;      // Clear from cursor to end of line
    Ok(())
}
