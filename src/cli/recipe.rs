use anyhow::{Result, bail};
use std::path::Path;

use crate::core::framework::{
    FrameworkId, FrameworkRegistry, FrameworkSource, ensure_frameworks_dir, example_ids,
    example_toml, recipe_contents,
};

pub fn run(id: Option<String>, force: bool) -> Result<()> {
    let Some(raw) = id.filter(|s| !s.trim().is_empty()) else {
        bail!(
            "recipe id required\n  usage: zapusk recipe init <id> [--force]\n  examples: {}\n  or a new id (writes a commented skeleton)",
            example_ids().join(", ")
        );
    };
    let id: FrameworkId = raw.parse()?;
    let dir = ensure_frameworks_dir()?;
    let dest = dir.join(format!("{}.toml", id.as_str()));

    let registry = FrameworkRegistry::load();
    let overrides_builtin = matches!(registry.source(id.as_str()), Some(FrameworkSource::Builtin));

    write_recipe_file(&dest, &recipe_contents(&id), force)?;

    println!("Wrote {}", dest.display());
    if overrides_builtin {
        println!("This overrides the builtin '{}' recipe.", id);
    } else if example_toml(id.as_str()).is_some() {
        println!("Example recipe — edit start.command if your project differs.");
    }
    println!(
        "Set type = \"{}\" on a project, or press `a` in the TUI.",
        id
    );
    Ok(())
}

pub fn write_recipe_file(path: &Path, contents: &str, force: bool) -> Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::framework::parse_spec;

    fn tmp_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "zapusk-recipe-init-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_id_lists_examples() {
        let err = run(None, false).unwrap_err().to_string();
        assert!(err.contains("rails"));
        assert!(err.contains("laravel"));
        assert!(err.contains("express"));
    }

    #[test]
    fn write_refuses_existing_without_force() {
        let dir = tmp_dir();
        let path = dir.join("rails.toml");
        std::fs::write(&path, "old\n").unwrap();
        let err = write_recipe_file(&path, "new\n", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("--force"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\n");
        write_recipe_file(&path, "new\n", true).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn written_example_and_skeleton_parse() {
        let dir = tmp_dir();
        let rails = dir.join("rails.toml");
        write_recipe_file(&rails, &recipe_contents(&FrameworkId::new("rails")), false).unwrap();
        let spec = parse_spec(&std::fs::read_to_string(&rails).unwrap()).unwrap();
        assert_eq!(spec.id, "rails");

        let next = dir.join("nextjs.toml");
        write_recipe_file(&next, &recipe_contents(&FrameworkId::new("nextjs")), false).unwrap();
        let spec = parse_spec(&std::fs::read_to_string(&next).unwrap()).unwrap();
        assert_eq!(spec.id, "nextjs");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
