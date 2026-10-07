// Waits until another job of this workflow run has uploaded an artifact, so that a job can start (runner, caches,
// project setup) before the job that makes its input has finished (.depot/workflows/bench.yml: the measuring jobs
// wait for the `build` job's tsrs-pgo-dist). Run through actions/github-script, the only kind of step that gets the
// runtime token the artifact service takes:
//
//   script: await require('./.github/scripts/wait-for-artifact.js')({core, name: 'tsrs-pgo-dist', failName: 'tsrs-build-failed'})
//
// Lists the run's artifacts every 5 s the way actions/download-artifact does (the results service's ListArtifacts,
// with the run's backend ids from the token's `Actions.Results` scope). Fails when `failName` appears (the producing
// job uploads it on failure) or after `timeoutMinutes`.
module.exports = async ({core, name, failName, timeoutMinutes = 45}) => {
  const token = process.env.ACTIONS_RUNTIME_TOKEN;
  const resultsUrl = process.env.ACTIONS_RESULTS_URL;
  if (!token || !resultsUrl) throw new Error('no ACTIONS_RUNTIME_TOKEN / ACTIONS_RESULTS_URL: run this through actions/github-script');
  const scopes = JSON.parse(Buffer.from(token.split('.')[1], 'base64url').toString()).scp.split(' ');
  const ids = scopes.find((s) => s.startsWith('Actions.Results:')).split(':');
  const url = new URL('/twirp/github.actions.results.api.v1.ArtifactService/ListArtifacts', resultsUrl).href;
  const body = JSON.stringify({workflow_run_backend_id: ids[1], workflow_job_run_backend_id: ids[2]});
  const start = Date.now();
  for (let i = 0; ; i++) {
    let names = [];
    try {
      const res = await fetch(url, {method: 'POST', headers: {'Content-Type': 'application/json', Authorization: `Bearer ${token}`}, body});
      if (res.ok) names = ((await res.json()).artifacts || []).map((a) => a.name);
      else core.info(`ListArtifacts: HTTP ${res.status}`);
    } catch (e) {
      core.info(`ListArtifacts: ${e.message}`);
    }
    const waited = Math.round((Date.now() - start) / 1000);
    if (names.includes(name)) {
      core.info(`${name} is there after ${waited} s`);
      return;
    }
    if (failName && names.includes(failName)) throw new Error(`${failName}: the job that makes ${name} failed`);
    if (waited > timeoutMinutes * 60) throw new Error(`${name} did not appear in ${timeoutMinutes} min`);
    if (i % 12 === 0) core.info(`waiting for ${name} (${waited} s)`);
    await new Promise((r) => setTimeout(r, 5000));
  }
};
