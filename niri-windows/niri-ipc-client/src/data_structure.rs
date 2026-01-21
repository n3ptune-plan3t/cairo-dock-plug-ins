/// Organized window data for rendering
#[derive(Debug, Clone)]
pub struct WorkspaceWindowsDisplay {
    pub workspace_id: u64,
    pub workspace_name: String,
    pub workspace_idx: u8,
    pub is_active: bool,
    pub is_focused: bool,
    pub windows_by_column: Vec<WindowColumn>,
}

#[derive(Debug, Clone)]
pub struct WindowColumn {
    pub column_idx: usize,
    pub windows: Vec<WindowDisplay>,
}

#[derive(Debug, Clone)]
pub struct WindowDisplay {
    pub id: u64,
    pub title: String,
    pub app_id:  Option<String>,
    pub is_focused: bool,
    pub is_floating: bool,
    pub column_idx: usize,
    pub tile_idx: usize,
    pub icon_path: String, // Computed icon path
}

impl WindowsByWorkspace {
    /// Get structured display data for all workspaces
    pub fn get_display_data(&self) -> Vec<WorkspaceWindowsDisplay> {
        let mut result = Vec::new();

        for workspace in self.get_workspaces_ordered() {
            let mut columns:  HashMap<usize, WindowColumn> = HashMap::new();
            let windows = self.get_workspace_windows(workspace.id);

            for window in windows {
                if let Some((col_idx, tile_idx)) = window. layout.pos_in_scrolling_layout {
                    let entry = columns
                        .entry(col_idx)
                        .or_insert_with(|| WindowColumn {
                            column_idx: col_idx,
                            windows: Vec::new(),
                        });

                    let icon_path = compute_icon_path(&window.app_id, &window.title);

                    entry.windows.push(WindowDisplay {
                        id:  window.id,
                        title: window.title. clone(),
                        app_id: window.app_id.clone(),
                        is_focused:  window.is_focused,
                        is_floating: window.is_floating,
                        column_idx: col_idx,
                        tile_idx: tile_idx,
                        icon_path,
                    });
                }
            }

            let mut columns_vec: Vec<_> = columns. into_values().collect();
            columns_vec.sort_by_key(|col| col.column_idx);

            result.push(WorkspaceWindowsDisplay {
                workspace_id: workspace.id,
                workspace_name: workspace
                    .name
                    . clone()
                    .unwrap_or_else(|| format!("Workspace {}", workspace. idx)),
                workspace_idx:  workspace.idx,
                is_active: workspace.is_active,
                is_focused: workspace. is_focused,
                windows_by_column: columns_vec,
            });
        }

        result
    }
}

fn compute_icon_path(app_id: &Option<String>, title: &str) -> String {
    // Later: use freedesktop icon theme to find app icon
    if let Some(app_id) = app_id {
        format! ("/usr/share/icons/hicolor/48x48/apps/{}. png", app_id)
    } else {
        "/usr/share/icons/hicolor/48x48/apps/application-x-executable.png".to_string()
    }
}
