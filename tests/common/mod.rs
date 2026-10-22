#![allow(dead_code)]
use anyhow::{Result, bail};
use serde_json::{Value, json};
use sm::gitlab::{CreateMr, Mr, MrApi, MrEdit};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};
use tempfile::TempDir;

pub fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
pub struct Repo {
    pub temp: TempDir,
    pub work: PathBuf,
    pub remote: PathBuf,
}
impl Repo {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let work = temp.path().join("work");
        let remote = temp.path().join("remote.git");
        assert!(
            Command::new("git")
                .args(["init", "--bare"])
                .arg(&remote)
                .output()
                .unwrap()
                .status
                .success()
        );
        assert!(
            Command::new("git")
                .args(["init", "-b", "main"])
                .arg(&work)
                .output()
                .unwrap()
                .status
                .success()
        );
        git(&work, &["config", "user.name", "SM Test"]);
        git(&work, &["config", "user.email", "sm@example.invalid"]);
        git(
            &work,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        let repo = Self { temp, work, remote };
        fs::create_dir_all(repo.work.join("resources/review")).unwrap();
        fs::write(repo.work.join("resources/review/SKILL.md"), "Approved").unwrap();
        repo.catalog(json!({"projects":{},"resources":[{"id":"review","kind":"skill","origin":"self","scope":"user","project":null,"payload":"resources/review"}]}));
        repo.publish("Initial catalog");
        git(&repo.remote, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        repo
    }
    pub fn catalog(&self, value: Value) {
        fs::write(
            self.work.join("catalog.json"),
            serde_json::to_vec_pretty(&value).unwrap(),
        )
        .unwrap();
    }
    pub fn publish(&self, message: &str) {
        git(&self.work, &["add", "--all"]);
        git(&self.work, &["commit", "-m", message]);
        git(&self.work, &["push", "origin", "main"]);
    }
    pub fn state(&self) -> sm::state::State {
        sm::state::State::open(self.temp.path().join("state/state.sqlite3")).unwrap()
    }
}
#[derive(Default)]
pub struct FakeMrApi {
    pub mrs: Mutex<Vec<Mr>>,
    pub offline: bool,
}
impl MrApi for FakeMrApi {
    fn opened(&self, branch: &str) -> Result<Vec<Mr>> {
        if self.offline {
            bail!("offline");
        }
        Ok(self
            .mrs
            .lock()
            .unwrap()
            .iter()
            .filter(|mr| mr.source_branch == branch)
            .cloned()
            .collect())
    }
    fn create(&self, input: &CreateMr) -> Result<Mr> {
        let mut records = self.mrs.lock().unwrap();
        let mr = Mr {
            iid: records.len() as u64 + 1,
            web_url: format!("https://git.example.invalid/mr/{}", records.len() + 1),
            source_branch: input.source_branch.clone(),
            target_branch: input.target_branch.clone(),
            title: input.edit.title.clone(),
            description: input.edit.description.clone(),
        };
        records.push(mr.clone());
        Ok(mr)
    }
    fn update(&self, iid: u64, input: &MrEdit) -> Result<()> {
        let mut records = self.mrs.lock().unwrap();
        let mr = records.iter_mut().find(|mr| mr.iid == iid).unwrap();
        mr.title = input.title.clone();
        mr.description = input.description.clone();
        Ok(())
    }
    fn get(&self, iid: u64) -> Result<Mr> {
        Ok(self
            .mrs
            .lock()
            .unwrap()
            .iter()
            .find(|mr| mr.iid == iid)
            .unwrap()
            .clone())
    }
}
