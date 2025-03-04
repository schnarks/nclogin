use std::{env, fs, ptr, process::{Command, Stdio}, mem::zeroed, ffi::CString, os::unix::{fs::chown, process::CommandExt}, path::{Path, PathBuf}};
use regex::Regex;
use libc::{self, gettimeofday, timeval, setutxent, utmpx, c_short, pututxline, endutxent, getutxline, c_char, sleep};

use crate::session::Session;
use crate::user::User;

fn prepare_environment(user: &User, session: &Session) {

    // Set user-specific environment variables
    env::set_var("SHELL", &user.shell);
    env::set_var("LOGNAME", &user.name);
    env::set_var("USER", &user.name);
    env::set_var("PWD", &user.homedir);
    env::set_var("HOME", &user.homedir);

    // Set session-specific environment variables
    env::set_var("XDG_SESSION_TYPE", session.session_type.to_string());
    env::set_var("XDG_CURRENT_DESKTOP", &session.name);
    env::set_var("XDG_DATA_HOME", format!("{}{}", &user.homedir,"/.local/share"));
    env::set_var("XDG_CONFIG_HOME", format!("{}{}", &user.homedir,"/.config"));
    env::set_var("XDG_CACHE_HOME", format!("{}{}", &user.homedir,"/.cache"));
    env::set_var("XDG_SESSION_CLASS", "user");
    env::set_var("XDG_RUNTIME_DIR", format!("/run/user/{}", user.uid));
    env::set_var("XDG_VTNR", get_tty_nr().unwrap_or(0).to_string());
    env::set_var("XDG_SEAT", get_seat_name());
    env::set_var("XDG_SESSION_ID", get_session_id(&get_tty_name(), &user.name).unwrap_or(0).to_string());

}

// Function to get the tty path (/dev/tty?)
pub fn get_tty_path() -> String {
    fs::read_link("/proc/self/fd/0").unwrap_or(PathBuf::from("/dev/tty?")).to_string_lossy().to_string()
}

// Function to get the tty name (tty?)
pub fn get_tty_name() -> String {
    let tty_path = get_tty_path();
    tty_path.trim_start_matches("/dev/").to_string()
}

// Function to get the tty number (?)
pub fn get_tty_nr() -> Option<i32> {
    let tty = get_tty_path();

    // Create a regex pattern to capture the number at the end of the string
    let re = Regex::new(r"(\d+)$").ok()?;

    if let Some(captures) = re.captures(&tty) {
        if let Some(tty_number_str) = captures.get(1) {
            return tty_number_str.as_str().parse().ok();
        }
    }
    None
}

// Function to get the first seat from loginctl or seat0
pub fn get_seat_name() -> String {
    // First attempt: Query loginctl to get the current seat of the user
    let output = Command::new("loginctl")
        .arg("seat-status")
        .output();

    if let Ok(output) = output {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(first_line) = stdout.lines().next() {
                return first_line.trim().to_string();
            }
        }
    }
    // Second attempt: Fallback to a default seat (seat0)
    "seat0".to_string()
}

// Function to get the session id of the tty and the username
pub fn get_session_id(tty_name: &str, user_name: &str) -> Option<i32> {
    // Run `loginctl list-sessions` to get a list of sessions
    let output = Command::new("loginctl")
        .arg("list-sessions")
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Iterate through each line of the output
    for line in stdout.lines() {
        let columns: Vec<&str> = line.split_whitespace().collect();

        // Ensure the line contains enough columns (SESSION, UID, USER, SEAT, TTY)
        if columns.len() > 4 && columns[4] == tty_name && columns[2] == user_name {
            // Return the session ID as i32
            return columns[0].parse::<i32>().ok();
        }
    }
    None
}

const LOGIN_PROCESS: c_short = 6;
const USER_PROCESS: c_short = 7;

fn add_utmpx_entry(username: &str, tty: &str, pid: i32) {
    unsafe {
        // Open utmpx file for updating
        setutxent();

        // Create a new utmpx entry
        let mut entry: utmpx = zeroed();
        entry.ut_type = USER_PROCESS;
        entry.ut_pid = pid;

        // Set the tty name (without `/dev/`)
        let ttyname = tty.trim_start_matches("/dev/");
        let tty_cstr = CString::new(ttyname).unwrap();

        // Ensure that the ttyname fits into the field
        let tty_len = std::cmp::min(tty_cstr.to_bytes().len(), entry.ut_line.len());
        ptr::copy_nonoverlapping(tty_cstr.as_ptr(), entry.ut_line.as_mut_ptr(), tty_len);
        ptr::copy_nonoverlapping(tty_cstr.as_ptr(), entry.ut_id.as_mut_ptr(), tty_len);

        // Set the username (ensure it fits in the buffer)
        let user_cstr = CString::new(username).unwrap();
        let user_len = std::cmp::min(user_cstr.to_bytes().len(), entry.ut_user.len());
        ptr::copy_nonoverlapping(user_cstr.as_ptr(), entry.ut_user.as_mut_ptr(), user_len);

        // Set timestamp
        let mut tv: timeval = zeroed();
        gettimeofday(&mut tv, ptr::null_mut());
        entry.ut_tv.tv_sec = tv.tv_sec as i32;
        entry.ut_tv.tv_usec = tv.tv_usec as i32;

        // Write entry to utmpx
        let result = pututxline(&mut entry);
        if result.is_null() {
            eprintln!("Failed to write to utmpx.");
        }
        // Close utmpx
        endutxent();
    }
}


fn remove_utmpx_entry() {
    unsafe {
        setutxent(); // Open utmpx file for reading & writing
        let tty_name = get_tty_name();
        let mut entry: utmpx = zeroed();
        let c_tty = CString::new(tty_name.clone()).unwrap();
        ptr::copy_nonoverlapping(c_tty.as_ptr(), entry.ut_line.as_mut_ptr(), tty_name.len());

        // Find the entry
        while let Some(current) = getutxline(&mut entry).as_mut() {
            if current.ut_line == entry.ut_line {
                current.ut_type = LOGIN_PROCESS;
                // Set ut_user to "LOGIN" (32 bytes, padded with 0)
                let user_bytes = b"LOGIN";
                let mut user_buf = [0 as c_char; 32]; // Use c_char for this array
                for (i, &byte) in user_bytes.iter().enumerate() {
                    user_buf[i] = byte as c_char;
                }
                current.ut_user = user_buf;

                pututxline(current); // Write update
                break;
            }
        }

        endutxent(); // Close utmpx
    }
}

fn change_tty_ownership(user_uid: u32, tty_path_str: &str) -> Result<(), nix::Error> {
    chown(Path::new(tty_path_str), Some(user_uid), None).unwrap();
    Ok(())
}

pub fn exec_session_as_user(user: &User, session: &Session) {
    // Get tty infos
    let tty_path = get_tty_path();
    let tty_name = get_tty_name();

    // Change tty ownership
    change_tty_ownership(user.uid as u32, &tty_path).expect("Couldn't change tty permissions");

    // Cd to user's home directory
    if let Err(e) = std::env::set_current_dir(&user.homedir) {
        eprintln!("Failed to change directory to home directory: {}", e);
    }

    prepare_environment(user, session);

    // Execute the session / shell
    let mut cmd = Command::new(&session.cmd);
    cmd
        .uid(user.uid as u32)
        .gid(user.gid as u32)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    match cmd.spawn() {
        Ok(mut child) => {
            let child_pid = child.id(); // Get the child process PID
            // Add utmp entry
            add_utmpx_entry(&user.name, &tty_name, child_pid as i32);
            let _ = child.wait(); // Wait for the child process to finish
        }
        Err(e) =>
            {
                eprintln!("Failed to execute command: {}", e);
                // sleep so error msg parsing is possible
                unsafe {
                    sleep(1);
                }
            }
    }
    // Reset tty permission
    change_tty_ownership(0, &tty_path).expect("Couldn't change tty permissions");

    // Clean up the utmp entry
    remove_utmpx_entry();
}
