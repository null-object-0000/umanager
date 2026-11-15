import fs from "node:fs";
import path from "node:path";
import { Makers } from "@edgeone/makers-sdk";

const directory = process.argv[2];
if (!directory || !fs.existsSync(path.join(directory, "mirror-status.json"))) throw new Error("Prepare the verified mirror publication first");
for (const name of ["TENCENT_CLOUD_SECRET_ID", "TENCENT_CLOUD_SECRET_KEY"]) if (!process.env[name]) throw new Error("Missing " + name);
const makers = new Makers({ credentials: { secretId: process.env.TENCENT_CLOUD_SECRET_ID, secretKey: process.env.TENCENT_CLOUD_SECRET_KEY }, zoneId: "zone-3xezk62mta64", region: "china" });
const projectId = "makers-toowqfa485mp";
const project = await makers.projects.get({ projectId });
if (project.area !== "global" || project.name !== "umanager-feed-global") throw new Error("Unexpected mirror project");
const deployment = await makers.deployments.deploy({ projectId, artifact: { directory }, env: "Production" });
const result = await makers.deployments.wait({ projectId, deploymentId: deployment.deploymentId, timeout: 300, pollInterval: 5 });
if (result.status !== "Success") throw new Error("Mirror deployment failed: " + result.status);
console.log(JSON.stringify({ deploymentId: deployment.deploymentId, status: result.status, site: "https://updates.snewbie.site/umanager/" }));
