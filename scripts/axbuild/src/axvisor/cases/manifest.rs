use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use serde::Deserialize;

pub(crate) const AXVISOR_TEST_SUITE_ROOT: &str = "test-suit/axvisor";
const CASE_MANIFEST_FILE: &str = "case.toml";
const VM_TEMPLATE_DIR: &str = "vm";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LoadedCase {
    pub(crate) case_dir: PathBuf,
    pub(crate) manifest: CaseManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SuiteManifest {
    pub(crate) name: String,
    pub(crate) arches: BTreeMap<String, SuiteArchManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SuiteArchManifest {
    pub(crate) cases: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct CaseManifest {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    pub(crate) arch: Vec<String>,
    pub(crate) timeout_secs: u64,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

pub(crate) fn load_cases_from_suite(
    workspace_root: &Path,
    suite_path: &Path,
    arch: &str,
) -> anyhow::Result<(SuiteManifest, Vec<LoadedCase>)> {
    let suite = load_suite_manifest(suite_path)?;
    let arch_entry = suite.arches.get(arch).ok_or_else(|| {
        anyhow!(
            "suite `{}` does not define cases for arch `{}`",
            suite.name,
            arch
        )
    })?;

    let suite_root = workspace_root.join(AXVISOR_TEST_SUITE_ROOT);
    let mut cases = Vec::with_capacity(arch_entry.cases.len());
    for case_ref in &arch_entry.cases {
        let case_dir = suite_root.join(case_ref);
        let loaded = load_case_from_dir(&case_dir).with_context(|| {
            format!(
                "failed to load case `{}` referenced by suite `{}`",
                case_ref, suite.name
            )
        })?;
        ensure_case_supports_arch(&loaded, arch)?;
        cases.push(loaded);
    }

    Ok((suite, cases))
}

pub(crate) fn load_case_from_dir(case_dir: &Path) -> anyhow::Result<LoadedCase> {
    let manifest_path = case_dir.join(CASE_MANIFEST_FILE);
    let manifest = load_case_manifest(&manifest_path)?;
    Ok(LoadedCase {
        case_dir: case_dir.to_path_buf(),
        manifest,
    })
}

pub(crate) fn discover_cases(root: &Path, arch: &str) -> anyhow::Result<Vec<LoadedCase>> {
    if !root.is_dir() {
        bail!("case discovery root does not exist: {}", root.display());
    }

    let mut cases = Vec::new();
    discover_cases_in_dir(root, arch, &mut cases)?;
    cases.sort_by(|left, right| {
        left.manifest
            .id
            .cmp(&right.manifest.id)
            .then_with(|| left.case_dir.cmp(&right.case_dir))
    });

    if cases.is_empty() {
        bail!(
            "no axvisor cases supporting arch `{arch}` were found under {}",
            root.display()
        );
    }

    Ok(cases)
}

fn discover_cases_in_dir(
    dir: &Path,
    arch: &str,
    cases: &mut Vec<LoadedCase>,
) -> anyhow::Result<()> {
    if should_skip_discovery_dir(dir) {
        return Ok(());
    }

    let manifest_path = dir.join(CASE_MANIFEST_FILE);
    if manifest_path.is_file() {
        let case = load_case_from_dir(dir)
            .with_context(|| format!("failed to load discovered case at {}", dir.display()))?;
        if case.manifest.arch.iter().any(|value| value == arch) {
            ensure_vm_template_exists(&case, arch)?;
            cases.push(case);
        }
        return Ok(());
    }

    let mut children = fs::read_dir(dir)
        .with_context(|| format!("failed to read discovery directory {}", dir.display()))?
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("failed to read entry under {}", dir.display()))?;
    children.sort_by_key(|entry| entry.path());

    for entry in children {
        let path = entry.path();
        if path.is_dir() {
            discover_cases_in_dir(&path, arch, cases)?;
        }
    }

    Ok(())
}

fn should_skip_discovery_dir(dir: &Path) -> bool {
    let Some(name) = dir.file_name().and_then(|name| name.to_str()) else {
        return false;
    };

    name.starts_with('.') || matches!(name, "common" | "example" | "suites")
}

fn ensure_vm_template_exists(case: &LoadedCase, arch: &str) -> anyhow::Result<()> {
    let template = case
        .case_dir
        .join(VM_TEMPLATE_DIR)
        .join(format!("{arch}.toml.in"));
    if template.is_file() {
        Ok(())
    } else {
        bail!(
            "case `{}` supports arch `{arch}` but VM template is missing: {}",
            case.manifest.id,
            template.display()
        )
    }
}

fn load_suite_manifest(path: &Path) -> anyhow::Result<SuiteManifest> {
    let manifest: SuiteManifest = read_toml(path)?;
    if manifest.arches.is_empty() {
        bail!(
            "suite manifest {} has no [arches.*] entries",
            path.display()
        );
    }
    Ok(manifest)
}

fn load_case_manifest(path: &Path) -> anyhow::Result<CaseManifest> {
    let manifest: CaseManifest = read_toml(path)?;
    validate_case_manifest(&manifest, path)?;
    Ok(manifest)
}

fn validate_case_manifest(manifest: &CaseManifest, path: &Path) -> anyhow::Result<()> {
    if manifest.id.trim().is_empty() {
        bail!("case manifest {} has empty `id`", path.display());
    }
    if manifest.arch.is_empty() {
        bail!(
            "case manifest {} must declare at least one arch",
            path.display()
        );
    }
    if manifest.timeout_secs == 0 {
        bail!(
            "case manifest {} must set `timeout_secs` > 0",
            path.display()
        );
    }
    Ok(())
}

pub(crate) fn ensure_case_supports_arch(case: &LoadedCase, arch: &str) -> anyhow::Result<()> {
    if case.manifest.arch.iter().any(|value| value == arch) {
        Ok(())
    } else {
        bail!(
            "case `{}` at {} does not support arch `{}`",
            case.manifest.id,
            case.case_dir.display(),
            arch
        )
    }
}

fn read_toml<T: for<'de> Deserialize<'de>>(path: &Path) -> anyhow::Result<T> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn load_case_manifest_requires_timeout() {
        let dir = tempdir().unwrap();
        let case_dir = dir.path().join("case");
        fs::create_dir_all(&case_dir).unwrap();

        fs::write(
            case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64"]
timeout_secs = 15
description = "example case"
"#,
        )
        .unwrap();

        let loaded = load_case_from_dir(&case_dir).unwrap();
        assert_eq!(loaded.manifest.id, "example.pass");
        assert_eq!(loaded.manifest.timeout_secs, 15);
        assert_eq!(loaded.manifest.description.as_deref(), Some("example case"));
    }

    #[test]
    fn load_case_manifest_rejects_zero_timeout() {
        let dir = tempdir().unwrap();
        let case_dir = dir.path().join("case");
        fs::create_dir_all(&case_dir).unwrap();

        fs::write(
            case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64"]
timeout_secs = 0
"#,
        )
        .unwrap();

        assert!(load_case_from_dir(&case_dir).is_err());
    }

    #[test]
    fn load_suite_manifest_resolves_selected_arch_cases() {
        let dir = tempdir().unwrap();
        let workspace_root = dir.path();
        let suite_root = workspace_root.join(AXVISOR_TEST_SUITE_ROOT);
        let case_dir = suite_root.join("example/pass-report");
        fs::create_dir_all(&case_dir).unwrap();
        fs::write(
            case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64", "x86_64"]
timeout_secs = 5
"#,
        )
        .unwrap();

        let suites_dir = suite_root.join("suites");
        fs::create_dir_all(&suites_dir).unwrap();
        let suite_path = suites_dir.join("examples.toml");
        fs::write(
            &suite_path,
            r#"
name = "examples"

[arches.aarch64]
cases = ["example/pass-report"]
"#,
        )
        .unwrap();

        let (suite, cases) = load_cases_from_suite(workspace_root, &suite_path, "aarch64").unwrap();
        assert_eq!(suite.name, "examples");
        assert_eq!(cases.len(), 1);
        assert_eq!(cases[0].manifest.id, "example.pass");
    }

    #[test]
    fn discover_cases_finds_supported_cases_and_skips_unsupported_arch() {
        let dir = tempdir().unwrap();
        let suite_root = dir.path().join(AXVISOR_TEST_SUITE_ROOT);

        let pass_dir = suite_root.join("cpu/pass-report");
        fs::create_dir_all(pass_dir.join("vm")).unwrap();
        fs::write(
            pass_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64", "riscv64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(pass_dir.join("vm/aarch64.toml.in"), "").unwrap();

        let skip_dir = suite_root.join("cpu/riscv-only");
        fs::create_dir_all(skip_dir.join("vm")).unwrap();
        fs::write(
            skip_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.riscv"
arch = ["riscv64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(skip_dir.join("vm/riscv64.toml.in"), "").unwrap();

        let discovered = discover_cases(&suite_root, "aarch64").unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].manifest.id, "example.pass");
    }

    #[test]
    fn discover_cases_rejects_supported_case_without_vm_template() {
        let dir = tempdir().unwrap();
        let suite_root = dir.path().join(AXVISOR_TEST_SUITE_ROOT);
        let case_dir = suite_root.join("cpu/pass-report");
        fs::create_dir_all(&case_dir).unwrap();
        fs::write(
            case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64"]
timeout_secs = 5
"#,
        )
        .unwrap();

        let err = discover_cases(&suite_root, "aarch64").unwrap_err();
        assert!(err.to_string().contains("VM template is missing"));
    }

    #[test]
    fn discover_cases_skips_helper_directories() {
        let dir = tempdir().unwrap();
        let suite_root = dir.path().join(AXVISOR_TEST_SUITE_ROOT);

        let common_case_dir = suite_root.join("common/helper");
        fs::create_dir_all(common_case_dir.join("vm")).unwrap();
        fs::write(
            common_case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "common.helper"
arch = ["aarch64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(common_case_dir.join("vm/aarch64.toml.in"), "").unwrap();

        let suite_case_dir = suite_root.join("suites/example");
        fs::create_dir_all(suite_case_dir.join("vm")).unwrap();
        fs::write(
            suite_case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "suite.helper"
arch = ["aarch64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(suite_case_dir.join("vm/aarch64.toml.in"), "").unwrap();

        let example_case_dir = suite_root.join("example/pass-report");
        fs::create_dir_all(example_case_dir.join("vm")).unwrap();
        fs::write(
            example_case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "example.pass"
arch = ["aarch64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(example_case_dir.join("vm/aarch64.toml.in"), "").unwrap();

        let real_case_dir = suite_root.join("cpu/pass-report");
        fs::create_dir_all(real_case_dir.join("vm")).unwrap();
        fs::write(
            real_case_dir.join(CASE_MANIFEST_FILE),
            r#"
id = "cpu.pass"
arch = ["aarch64"]
timeout_secs = 5
"#,
        )
        .unwrap();
        fs::write(real_case_dir.join("vm/aarch64.toml.in"), "").unwrap();

        let discovered = discover_cases(&suite_root, "aarch64").unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].manifest.id, "cpu.pass");
    }
}
