use std::{
    env::current_dir,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};
use structopt::StructOpt;
use toml_edit::{decorated, Document, Item, Value};

enum PatchTarget {
    Crates,
    Git(String),
    Custom(String),
}

impl PatchTarget {
    /// Returns the patch target in a toml compatible format.
    fn as_string(&self) -> String {
        match self {
            Self::Crates => "crates-io".into(),
            Self::Git(url) => format!("\"{}\"", url),
            Self::Custom(custom) => format!("\"{}\"", custom),
        }
    }
}

/// `patch` subcommand options.
#[derive(Debug, StructOpt)]
pub struct Patch {
    /// The path to the project where the patch section should be added.
    ///
    /// If not given, the current directory will be taken.
    ///
    /// The patches will be added to the cargo workspace `Cargo.toml` file.
    #[structopt(long)]
    path: Option<PathBuf>,

    /// The workspace that should be scanned and added to the patch section.
    ///
    /// This will execute `cargo metadata` in the given workspace and add
    /// all packages of this workspace to the patch section.
    #[structopt(long)]
    crates_to_patch: PathBuf,

    /// The patch target that should be used.
    ///
    /// The target is `[patch.TARGET]` in the final `Cargo.toml`.
    #[structopt(
        long,
        conflicts_with_all = &[ "crates", "substrate", "polkadot" ]
    )]
    target: Option<String>,

    /// Use the official Substrate repo as patch target.
    #[structopt(
        long,
        short = "s",
        conflicts_with_all = &[ "target", "polkadot", "crates" ]
    )]
    substrate: bool,

    /// Use the official Polkadot repo as patch target.
    #[structopt(
        long,
        short = "p",
        conflicts_with_all = &[ "target", "substrate", "crates" ]
    )]
    polkadot: bool,

    /// Use `crates.io` as patch target.
    #[structopt(
        long,
        conflicts_with_all = &[ "target", "substrate", "polkadot" ]
    )]
    crates: bool,
}

impl Patch {
    /// Run this subcommand.
    pub fn run(self) -> Result<(), String> {
        let patch_target = self.patch_target()?;

        let path = self.path.map(Ok).unwrap_or_else(|| {
            current_dir().map_err(|e| format!("Working directory is invalid: {:?}", e))
        })?;

        // Get the path to the `Cargo.toml` where we need to add the patches
        let cargo_toml_to_patch = workspace_root_package(&path)?;

        add_patches_for_packages(
            &cargo_toml_to_patch,
            &patch_target,
            workspace_packages(&self.crates_to_patch)?,
        )
    }

    fn patch_target(&self) -> Result<PatchTarget, String> {
        if let Some(ref custom) = self.target {
            Ok(PatchTarget::Custom(custom.clone()))
        } else if self.substrate {
            Ok(PatchTarget::Git(
                "https://github.com/paritytech/substrate".into(),
            ))
        } else if self.polkadot {
            Ok(PatchTarget::Git(
                "https://github.com/paritytech/polkadot".into(),
            ))
        } else if self.crates {
            Ok(PatchTarget::Crates)
        } else {
            Err("You need to pass `--target`, `--substrate`, `--polkadot` or `--crates`!".into())
        }
    }
}

fn workspace_root_package(path: &Path) -> Result<PathBuf, String> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(path)
        .exec()
        .map_err(|e| {
            format!(
                "Failed to get cargo metadata for workspace `{}`: {:?}",
                path.display(),
                e
            )
        })?;

    Ok(metadata.workspace_root.join("Cargo.toml"))
}

/// Returns all package names of the given `workspace`.
fn workspace_packages(
    workspace: &Path,
) -> Result<impl Iterator<Item = cargo_metadata::Package>, String> {
    let metadata = cargo_metadata::MetadataCommand::new()
        .current_dir(workspace)
        .exec()
        .map_err(|e| {
            format!(
                "Failed to get cargo metadata for workspace `{}`: {:?}",
                workspace.display(),
                e
            )
        })?;

    Ok(metadata
        .workspace_members
        .clone()
        .into_iter()
        .map(move |p| metadata[&p].clone()))
}

fn add_patches_for_packages(
    cargo_toml: &Path,
    patch_target: &PatchTarget,
    mut packages: impl Iterator<Item = cargo_metadata::Package>,
) -> Result<(), String> {
    let content = fs::read_to_string(cargo_toml)
        .map_err(|e| format!("Could not read `{}`: {:?}", cargo_toml.display(), e))?;
    let mut doc = Document::from_str(&content).map_err(|e| {
        format!(
            "Failed to parse `{}` as `Cargo.toml`: {:?}",
            cargo_toml.display(),
            e
        )
    })?;

    let patch_table = doc
        .as_table_mut()
        .entry("patch")
        .or_insert(Item::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| "Patch table isn't a toml table!")?;

    patch_table.set_implicit(true);

    let patch_target_table = patch_table
        .entry(&patch_target.as_string())
        .or_insert(Item::Table(Default::default()))
        .as_table_mut()
        .ok_or_else(|| "Patch target table isn't a toml table!")?;

    packages.try_for_each(|mut p| {
        println!("Adding patch for `{}`.", p.name);

        let patch = patch_target_table
            .entry(&p.name)
            .or_insert(Item::Value(Value::InlineTable(Default::default())))
            .as_inline_table_mut()
            .ok_or_else(|| format!("Patch entry for `{}` isn't an inline table!", p.name))?;

        let path = if p.manifest_path.ends_with("Cargo.toml") {
            p.manifest_path.pop();
            p.manifest_path
        } else {
            p.manifest_path
        };

        *patch.get_or_insert("path", "") = decorated(path.display().to_string().into(), " ", " ");
        Ok::<_, String>(())
    })?;

    fs::write(&cargo_toml, doc.to_string_in_original_order())
        .map_err(|e| format!("Failed to write to `{}`: {:?}", cargo_toml.display(), e))
}
