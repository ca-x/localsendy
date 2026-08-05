use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};

pub async fn prepare_root(root: &Path) -> Result<PathBuf> {
    tokio::fs::create_dir_all(root)
        .await
        .with_context(|| format!("failed to create storage root {}", root.display()))?;
    tokio::fs::canonicalize(root)
        .await
        .with_context(|| format!("failed to resolve storage root {}", root.display()))
}

pub async fn resolve_subdirectory(root: &Path, value: &str) -> Result<(String, PathBuf)> {
    let root = prepare_root(root).await?;
    let relative = normalize_relative(value)?;
    let mut current = root.clone();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("save directory must stay inside the configured root");
        };
        current.push(name);
        let metadata = tokio::fs::symlink_metadata(&current)
            .await
            .with_context(|| format!("save directory does not exist: {}", current.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            bail!("save directory must contain only real directories");
        }
    }
    Ok((relative_to_string(&relative), current))
}

pub async fn create_subdirectory(
    root: &Path,
    parent: &str,
    name: &str,
) -> Result<(String, PathBuf)> {
    let (parent_relative, parent_path) = resolve_subdirectory(root, parent).await?;
    let name = validate_directory_name(name)?;
    let path = parent_path.join(&name);
    tokio::fs::create_dir(&path)
        .await
        .with_context(|| format!("failed to create directory {}", path.display()))?;
    let relative = if parent_relative.is_empty() {
        name
    } else {
        format!("{parent_relative}/{name}")
    };
    Ok((relative, path))
}

pub async fn list_subdirectories(root: &Path, value: &str) -> Result<(String, Vec<String>)> {
    let (relative, path) = resolve_subdirectory(root, value).await?;
    let mut entries = tokio::fs::read_dir(&path)
        .await
        .with_context(|| format!("failed to read directory {}", path.display()))?;
    let mut directories = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && let Some(name) = entry.file_name().to_str()
        {
            directories.push(name.to_owned());
        }
    }
    directories.sort_by_key(|name| name.to_lowercase());
    Ok((relative, directories))
}

fn normalize_relative(value: &str) -> Result<PathBuf> {
    let value = value.trim().replace('\\', "/");
    if value.chars().any(char::is_control) || value.len() > 1024 {
        bail!("save directory is invalid");
    }
    let path = Path::new(&value);
    if path.is_absolute() {
        bail!("save directory must be relative to the configured root");
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!("save directory must not contain '.' or '..'");
        }
    }
    Ok(path.to_path_buf())
}

fn validate_directory_name(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().any(char::is_control)
        || value.chars().count() > 128
        || value.contains(['/', '\\'])
        || matches!(value, "." | "..")
    {
        bail!("directory name is invalid");
    }
    Ok(value.to_owned())
}

fn relative_to_string(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn creates_lists_and_resolves_only_beneath_root() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("downloads");
        let (relative, path) = create_subdirectory(&root, "", "Projects").await.unwrap();
        assert_eq!(relative, "Projects");
        assert_eq!(
            resolve_subdirectory(&root, "Projects").await.unwrap().1,
            path
        );
        assert_eq!(
            list_subdirectories(&root, "").await.unwrap().1,
            vec!["Projects"]
        );
    }

    #[tokio::test]
    async fn rejects_parent_paths_and_symlink_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("downloads");
        prepare_root(&root).await.unwrap();
        assert!(resolve_subdirectory(&root, "../outside").await.is_err());

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(directory.path(), root.join("escape")).unwrap();
            assert!(resolve_subdirectory(&root, "escape").await.is_err());
        }
    }
}
