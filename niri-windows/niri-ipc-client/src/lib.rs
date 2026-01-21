//! Rust FFI client for Niri IPC (using official niri-ipc crate v25.11)
//!
//! This library wraps the official niri-ipc types and exposes them to C code via FFI. 

use std::collections::HashMap;
use std::sync: :{Arc, Mutex};
use std::path::PathBuf;
use std::io;
use anyhow::Result;
use niri_ipc::socket::Socket;
use niri_ipc: :{Event, Window, Workspace, WindowLayout, Request};

// ============================================================================
// DATA ORGANIZATION
// ============================================================================

/// Organized window state by workspace
#[derive(Debug, Clone)]
pub struct WindowsByWorkspace {
    pub workspaces: HashMap<u64, Workspace>,
    pub windows: HashMap<u64, Window>,
}

impl WindowsByWorkspace {
    pub fn new() -> Self {
        Self {
            workspaces: HashMap::new(),
            windows: HashMap::new(),
        }
    }

    /// Get all tiled windows for a workspace, sorted by column then tile (left-to-right, top-to-bottom)
    pub fn get_workspace_windows(&self, workspace_id: u64) -> Vec<&Window> {
        let mut windows:  Vec<&Window> = self
            .windows
            .values()
            .filter(|w| w.workspace_id == Some(workspace_id) && ! w.is_floating)
            .collect();

        // Sort by column index (X position), then by tile index within column (Y position)
        windows.sort_by(|a, b| {
            let pos_a = a.layout.pos_in_scrolling_layout. unwrap_or((999, 999));
            let pos_b = b.layout.pos_in_scrolling_layout.unwrap_or((999, 999));
            
            match pos_a.0.cmp(&pos_b.0) {
                std::cmp::Ordering::Equal => pos_a.1.cmp(&pos_b.1),
                other => other,
            }
        });

        windows
    }

    /// Get all floating windows for a workspace
    pub fn get_workspace_floating_windows(&self, workspace_id: u64) -> Vec<&Window> {
        self.windows
            .values()
            .filter(|w| w.workspace_id == Some(workspace_id) && w.is_floating)
            .collect()
    }

    /// Get workspaces in index order
    pub fn get_workspaces_ordered(&self) -> Vec<&Workspace> {
        let mut workspaces: Vec<&Workspace> = self.workspaces.values().collect();
        workspaces.sort_by_key(|ws| ws.idx);
        workspaces
    }

    /// Get the currently focused/active workspace
    pub fn get_active_workspace(&self) -> Option<&Workspace> {
        self. workspaces.values().find(|ws| ws.is_focused)
    }

    /// Apply an IPC event to update internal state
    pub fn apply_event(&mut self, event: &Event) {
        match event {
            Event::WorkspacesChanged { workspaces } => {
                self.workspaces. clear();
                for ws in workspaces {
                    self.workspaces.insert(ws.id, ws.clone());
                }
            }
            Event::WorkspaceActivated { id, focused:  _ } => {
                // Mark all workspaces as not focused
                for ws in self. workspaces.values_mut() {
                    ws.is_focused = false;
                }
                // Mark the activated workspace as focused
                if let Some(ws) = self.workspaces.get_mut(id) {
                    ws.is_focused = true;
                }
            }
            Event::WorkspaceActiveWindowChanged {
                workspace_id,
                active_window_id,
            } => {
                if let Some(ws) = self.workspaces.get_mut(workspace_id) {
                    ws.active_window_id = *active_window_id;
                }
            }
            Event:: WindowsChanged { windows } => {
                self.windows.clear();
                for win in windows {
                    self. windows.insert(win.id, win.clone());
                }
            }
            Event::WindowOpenedOrChanged { window } => {
                self.windows.insert(window.id, window.clone());
            }
            Event::WindowClosed { id } => {
                self.windows.remove(id);
            }
            Event::WindowFocusChanged { id } => {
                // Mark all windows as not focused
                for win in self.windows.values_mut() {
                    win.is_focused = false;
                }
                // Mark the focused window
                if let Some(id) = id {
                    if let Some(win) = self.windows.get_mut(id) {
                        win.is_focused = true;
                    }
                }
            }
            Event::WindowLayoutsChanged { changes } => {
                for (id, layout) in changes {
                    if let Some(win) = self.windows.get_mut(id) {
                        win.layout = layout.clone();
                    }
                }
            }
            // Other events don't affect our state
            _ => {}
        }
    }
}

// ============================================================================
// IPC CLIENT
// ============================================================================

pub struct NiriIPCClient {
    socket_path: PathBuf,
    state: Arc<Mutex<WindowsByWorkspace>>,
}

impl NiriIPCClient {
    /// Create a new Niri IPC client, auto-detecting the socket path
    pub fn new() -> Result<Self> {
        let socket_path = Self::find_socket()?;
        let state = Arc::new(Mutex::new(WindowsByWorkspace::new()));

        Ok(Self { socket_path, state })
    }

    /// Find the Niri IPC socket from environment
    fn find_socket() -> Result<PathBuf> {
        // Try to use NIRI_SOCKET environment variable first
        if let Ok(path) = std::env::var("NIRI_SOCKET") {
            return Ok(PathBuf::from(path));
        }

        // Fallback:  search in XDG_RUNTIME_DIR
        let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
            .unwrap_or_else(|_| "/tmp".to_string());
        let wayland_display = std::env::var("WAYLAND_DISPLAY")
            .unwrap_or_else(|_| "wayland-0".to_string());

        let dir = PathBuf::from(&runtime_dir);
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if let Some(name) = path.file_name() {
                if let Some(name_str) = name.to_str() {
                    if name_str. starts_with(&format!("niri. {}", wayland_display)) {
                        return Ok(path);
                    }
                }
            }
        }

        anyhow::bail!(
            "Niri IPC socket not found. Make sure Niri is running with WAYLAND_DISPLAY={}",
            wayland_display
        )
    }

    pub fn get_state(&self) -> Arc<Mutex<WindowsByWorkspace>> {
        self.state.clone()
    }

    /// Fetch initial state (workspaces and windows)
    pub fn fetch_initial_state(&self) -> Result<()> {
        // Connect and request workspaces
        let mut socket = niri_ipc::socket::Socket::connect_to(&self.socket_path)?;
        let reply = socket.send(Request:: Workspaces)?;
        
        if let Ok(niri_ipc::Response::Workspaces(workspaces)) = reply {
            let mut state = self.state.lock().unwrap();
            state.workspaces. clear();
            for ws in workspaces {
                state.workspaces.insert(ws.id, ws);
            }
        }

        // Connect and request windows
        let mut socket = niri_ipc::socket::Socket::connect_to(&self.socket_path)?;
        let reply = socket.send(Request::Windows)?;
        
        if let Ok(niri_ipc::Response::Windows(windows)) = reply {
            let mut state = self.state.lock().unwrap();
            state. windows.clear();
            for win in windows {
                state.windows.insert(win.id, win);
            }
        }

        Ok(())
    }

    /// Start listening to the event stream (blocking)
    pub async fn start_event_stream(&self) -> Result<()> {
        let mut socket = niri_ipc:: socket::Socket::connect_to(&self.socket_path)?;
        
        // Request event stream
        let _reply = socket.send(Request:: EventStream)?;

        // Read events continuously
        let mut read_event = socket.read_events();
        loop {
            match read_event() {
                Ok(event) => {
                    let mut state = self.state.lock().unwrap();
                    state.apply_event(&event);
                    tracing:: debug!("Event received: {:?}", event);
                }
                Err(e) => {
                    tracing:: error!("Error reading event:  {}", e);
                    break;
                }
            }
        }

        Ok(())
    }
}

// ============================================================================
// FFI EXPORTS
// ============================================================================

use std::ffi: :{CStr, CString};
use std::os::raw:: c_char;

pub struct NiriClientHandle {
    client: NiriIPCClient,
    runtime:  tokio::runtime::Runtime,
}

/// Create a new Niri IPC client handle
#[no_mangle]
pub extern "C" fn niri_client_new() -> *mut NiriClientHandle {
    match NiriIPCClient::new() {
        Ok(client) => match tokio::runtime::Runtime::new() {
            Ok(runtime) => Box::into_raw(Box:: new(NiriClientHandle { client, runtime })),
            Err(e) => {
                tracing::error!("Failed to create Tokio runtime: {}", e);
                std::ptr::null_mut()
            }
        },
        Err(e) => {
            tracing::warn!("Failed to create Niri IPC client: {}", e);
            std::ptr::null_mut()
        }
    }
}

/// Free a Niri IPC client handle
#[no_mangle]
pub extern "C" fn niri_client_free(handle: *mut NiriClientHandle) {
    if ! handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

/// Fetch initial workspaces and windows state
#[no_mangle]
pub extern "C" fn niri_client_fetch_initial_state(handle: *mut NiriClientHandle) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        match (*handle).client.fetch_initial_state() {
            Ok(_) => 1,
            Err(e) => {
                tracing::error!("Failed to fetch initial state:  {}", e);
                0
            }
        }
    }
}

/// Start the event stream (blocking call, should run in a thread)
#[no_mangle]
pub extern "C" fn niri_client_start_event_stream(handle: *mut NiriClientHandle) -> u8 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        match (*handle)
            .runtime
            .block_on((*handle).client.start_event_stream())
        {
            Ok(_) => 1,
            Err(e) => {
                tracing::error!("Event stream error: {}", e);
                0
            }
        }
    }
}

// ============================================================================
// FFI DATA STRUCTURES
// ============================================================================

#[repr(C)]
pub struct CWindow {
    pub id: u64,
    pub title: *const c_char,
    pub app_id: *const c_char,
    pub is_focused: u8,
    pub is_floating: u8,
    pub column_idx: u32,
    pub tile_idx: u32,
}

#[repr(C)]
pub struct CWorkspace {
    pub id: u64,
    pub idx: u8,
    pub name: *const c_char,
    pub is_focused: u8,
    pub is_active: u8,
    pub window_count: u32,
}

// ============================================================================
// FFI QUERY FUNCTIONS
// ============================================================================

/// Get the number of workspaces
#[no_mangle]
pub extern "C" fn niri_get_workspaces_count(handle: *mut NiriClientHandle) -> u32 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let state = (*handle).client.get_state();
        let state = state.lock().unwrap();
        state.workspaces.len() as u32
    }
}

/// Get the ID of the currently active (focused) workspace
#[no_mangle]
pub extern "C" fn niri_get_active_workspace_id(handle: *mut NiriClientHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let state = (*handle).client.get_state();
        let state = state.lock().unwrap();
        state
            .get_active_workspace()
            .map(|ws| ws.id)
            .unwrap_or(0)
    }
}

/// Get windows for a specific workspace
/// 
/// # Arguments
/// * `handle` - Niri client handle
/// * `workspace_id` - Workspace ID to query
/// * `out_windows` - Pointer to array to fill with windows
/// * `max_windows` - Maximum number of windows to return
///
/// # Returns
/// Number of windows returned
#[no_mangle]
pub extern "C" fn niri_get_windows_for_workspace(
    handle: *mut NiriClientHandle,
    workspace_id: u64,
    out_windows: *mut CWindow,
    max_windows: u32,
) -> u32 {
    if handle.is_null() || out_windows.is_null() {
        return 0;
    }

    unsafe {
        let state = (*handle).client.get_state();
        let state = state.lock().unwrap();
        let windows = state.get_workspace_windows(workspace_id);

        let mut count = 0u32;
        for (i, window) in windows.iter().enumerate() {
            if i >= max_windows as usize {
                break;
            }

            let (col_idx, tile_idx) = window.layout.pos_in_scrolling_layout.unwrap_or((0, 0));

            let title = CString::new(window.title.clone().unwrap_or_default())
                .unwrap_or_else(|_| CString::new("").unwrap());
            let app_id = window
                .app_id
                . as_ref()
                .and_then(|s| CString::new(s. clone()).ok());

            let c_window = CWindow {
                id:  window.id,
                title: title.as_ptr(),
                app_id: app_id. as_ref().map(|s| s.as_ptr()).unwrap_or_else(std::ptr::null),
                is_focused: if window.is_focused { 1 } else { 0 },
                is_floating: if window.is_floating { 1 } else { 0 },
                column_idx:  col_idx as u32,
                tile_idx: tile_idx as u32,
            };

            *out_windows.add(i) = c_window;
            count += 1;
        }

        count
    }
}

/// Get the name of a workspace
#[no_mangle]
pub extern "C" fn niri_get_workspace_name(
    handle: *mut NiriClientHandle,
    workspace_id:  u64,
    name_out: *mut c_char,
    name_len: usize,
) -> u8 {
    if handle.is_null() || name_out.is_null() || name_len == 0 {
        return 0;
    }

    unsafe {
        let state = (*handle).client.get_state();
        let state = state.lock().unwrap();

        if let Some(ws) = state.workspaces.get(&workspace_id) {
            let name = ws
                .name
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or(&format!("Workspace {}", ws.idx));
            
            let bytes = name.as_bytes();
            if bytes.len() + 1 < name_len {
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    name_out as *mut u8,
                    bytes.len(),
                );
                *(name_out.add(bytes.len())) = 0;
                return 1;
            }
        }
        0
    }
}

/// Get the total number of windows across all workspaces
#[no_mangle]
pub extern "C" fn niri_get_total_windows(handle: *mut NiriClientHandle) -> u32 {
    if handle.is_null() {
        return 0;
    }
    unsafe {
        let state = (*handle).client.get_state();
        let state = state.lock().unwrap();
        state.windows.len() as u32
    }
}
