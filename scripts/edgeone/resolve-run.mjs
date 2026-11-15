import fs from "node:fs";
import path from "node:path";
import {fileURLToPath} from "node:url";
export async function resolvePublication(api, repository) {
  // The dispatch job can still be finishing while its already completed merge
  // job has published Pages. Trust that publication, not overall run status.
  const runs = await api("actions/workflows/update-feed.yml/runs?branch=main&per_page=10");
  for (const run of runs.workflow_runs) {
    if (!Number.isSafeInteger(run.id) || run.id <= 0 || run.head_branch !== "main" || run.head_repository?.full_name !== repository || run.path !== ".github/workflows/update-feed.yml") continue;
    const jobs = await api(`actions/runs/${run.id}/jobs?per_page=100`);
    // A scrape group may fail while merge successfully publishes signed fallback
    // data. Mirror that successful Pages publication, not just all-green runs.
    if (!jobs.jobs.some(j => j.name === "merge" && j.conclusion === "success" && j.steps.some(s => s.name === "Deploy to GitHub Pages" && s.conclusion === "success"))) continue;
    if (!Number.isSafeInteger(run.run_attempt) || run.run_attempt < 1) throw new Error("Invalid source attempt");
    const name = `github-pages-${run.id}-${run.run_attempt}`;
    const artifacts = await api(`actions/runs/${run.id}/artifacts?per_page=100`);
    if (!artifacts.artifacts.some(a => a.name === name && !a.expired)) throw new Error("Latest published Pages artifact is unavailable");
    return { id: run.id, name };
  }
  throw new Error("No successful main-branch Pages publication");
}

async function main() {
const repository = process.env.GITHUB_REPOSITORY;
if (repository !== "null-object-0000/umanager") throw new Error("Unexpected repository");
const event = JSON.parse(fs.readFileSync(process.env.GITHUB_EVENT_PATH, "utf8"));
async function api(route) {
  const r = await fetch(`https://api.github.com/repos/${repository}/${route}`, { headers: { Authorization: "Bearer " + process.env.GITHUB_TOKEN, Accept: "application/vnd.github+json" } });
  if (!r.ok) throw new Error("GitHub API failed: " + r.status);
  return r.json();
}
if (process.env.GITHUB_EVENT_NAME === "workflow_run" && (event.workflow_run?.head_branch !== "main" || event.workflow_run?.head_repository?.full_name !== repository)) throw new Error("Untrusted publication event");
// Resolve at execution time so an older queued completion cannot roll the mirror back.
const {id, name} = await resolvePublication(api, repository);
fs.appendFileSync(process.env.GITHUB_OUTPUT, `run_id=${id}\nartifact_name=${name}\n`);
console.log("Mirroring the exact signed Pages artifact from run " + id);
}
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
