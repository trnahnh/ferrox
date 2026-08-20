# Cloud Deployment — Measured Results

This document records what was actually deployed for `docs/SYSTEM_DESIGN.md` §13 (the decoupled UDP subscriber / gap-recovery path), the real numbers measured against it, and the rough edges hit along the way. See §13 for the architecture rationale (Fargate over Lambda, unicast relay over multicast-in-VPC). This is the receipts document — real AWS account, real resources, real bill.

## What was deployed

- `examples/subscriber.rs` (dual-mode: multicast on-prem / unicast in cloud) containerized via `docker/subscriber.Dockerfile`, pushed to a private ECR repository.
- `infra/` (Terraform): ECR repo, ECS cluster + Fargate task (0.25 vCPU / 0.5 GB) + service, Network Load Balancer with a UDP:9001 listener → `ip`-target-type target group, security groups, IAM roles, a 1-day-retention CloudWatch log group.
- Deployed to the account's default VPC in `us-east-1`, across its 6 default subnets, using an existing least-recently-created Terraform deploy IAM user already provisioned for this AWS account.
- No live exchange traffic crossed a real `examples/relay.rs` in this exercise — see "What wasn't tested" below.

## Cost — measured

Pulled live from the AWS Pricing API for `us-east-1` at deployment time (not textbook numbers):

| Component | Rate | Hourly (this config) |
| --- | --- | --- |
| Fargate vCPU (0.25 vCPU) | $0.04048 / vCPU-hr | $0.0101 |
| Fargate memory (0.5 GB) | $0.004445 / GB-hr | $0.0022 |
| NLB base | $0.0225 / hr | $0.0225 |
| NLB LCU (≈1 LCU at this traffic level) | $0.006 / LCU-hr | $0.0060 |
| CloudWatch Logs ingestion (test volume: a few KB) | $0.50 / GB | ~$0.00001 |
| ECR image storage (~80 MB) | $0.10 / GB-mo | ~$0.0001 / hr equiv |
| **Total, running** | | **≈ $0.041 / hr ≈ $29.50 / mo if left on 24/7** |

**The NLB is the dominant line item** — $0.0285/hr of the $0.041/hr total (70%) is the load balancer, not the compute. For a single low-traffic subscriber, the NLB costs more than twice what the Fargate task itself costs. This is the single most important cost fact this exercise surfaced: **the decision to put a managed load balancer in front of this workload costs more than the workload.**

This deployment ran for ~40 minutes end-to-end (build, deploy, test, measure) before teardown, for a total incurred cost of **≈ $0.03** for the entire exercise.

### EC2 always-on comparison

A `t4g.nano` (Graviton, $0.0042/hr on-demand, `us-east-1`) running the identical binary directly, with a public IP and no load balancer:

| Component | Hourly |
| --- | --- |
| t4g.nano compute | $0.0042 |
| 8 GB gp3 EBS (amortized) | ~$0.0009 |
| **Total** | **≈ $0.0051 / hr ≈ $3.70 / mo** |

That's **~8x cheaper** than Fargate+NLB — almost entirely because it skips the NLB fee by exposing the instance's public IP directly. This is not an apples-to-apples comparison, though: the EC2 box gets no managed health checking, no rolling deploys, no automatic task replacement on crash (you'd wire up `systemd` + a restart policy yourself), and if you add an NLB in front of it for HA or a stable DNS name, most of the cost gap closes immediately — you'd be back to paying the same $0.0225+/hr NLB fee EC2 was avoiding. The honest conclusion is narrower than "EC2 is cheaper": **for a single subscriber with no HA requirement, skip the load balancer entirely (point a relay or client straight at the box's public IP); Fargate's real advantage — managed restarts, rolling deploys — only pays for itself once you actually need those properties.**

## Cold start — measured

Timestamps from `aws ecs describe-tasks` on a real task launch (`ecs update-service --force-new-deployment` → healthy):

| Stage | Elapsed from task creation |
| --- | --- |
| Task created → ENI attached (`connectivityAt`) | ~3.7s |
| → image pull starts (`pullStartedAt`) | ~11.0s |
| → image pull completes (`pullStoppedAt`, ~80MB image) | ~14.1s |
| → container started (`startedAt`) | ~22.9s |
| → NLB marks target healthy (2 consecutive TCP checks, 10s interval) | ~45–55s (additional) |

**Total cold start, task-launch-request to serving healthy traffic: ~45–70 seconds.** Nearly half of that is the NLB health check's own convergence time (`healthy_threshold=2 × interval=10s`, plus the "initial" grace period before the first check even fires), not container startup. This is the real, measured cost of the health-check design forced by §13.3 (a TCP stub health port bolted onto a UDP-only service) — it's not just an architectural footnote, it visibly slows every deployment and every auto-scaling event.

## Gap-detection latency vs. in-process

This is the number that turned out **not** to be honestly measurable from this environment, and it's worth explaining why rather than reporting a fabricated figure:

- The obvious approach — embed a send timestamp in each synthetic packet, compare against the CloudWatch log timestamp when it's decoded — requires the test client's clock and AWS's clocks to agree to sub-second precision. This dev sandbox's local clock was measured (`w32tm /stripchart`) to be **~785ms off from NTP**, which swamps any real network/processing latency by two to three orders of magnitude. Reporting a number from that comparison would have been fiction wearing a precision label.
- ICMP to the NLB's resolved IP (a clean, single-clock RTT proxy) was blocked outbound from this sandbox, so a raw network-transit baseline wasn't obtainable either.
- What **is** measurable cleanly, because both timestamps come from AWS's own clock domain (the container's `awslogs` log driver and CloudWatch's ingestion pipeline), is **CloudWatch Logs delivery latency**: the gap between when the subscriber process emitted a log line and when it became queryable in CloudWatch. Measured across 26 synthetic `ExecutionReport` packets: **275ms–4.9s**, driven by the `awslogs` driver's batching (multiple lines are batched and flushed together, which is why many events share an identical `ingestionTime`).

**Honest bottom line**: the in-process hot path is 500ns P99 (Phase 7, §12). Nothing about the cloud subscriber path — UDP transit, NLB, Fargate, and especially CloudWatch Logs as the observability sink — belongs anywhere near that number, and no measurement here should be read as suggesting otherwise. The one number actually pinned down (CloudWatch delivery: hundreds of ms to several seconds) is itself an argument against depending on CloudWatch Logs for anything gap-detection-time-sensitive; a real deployment would have the subscriber act on gaps directly in-process (as it already does — the gap check happens before the log line is even written) and use CloudWatch purely for after-the-fact observability, not as part of the detection path itself.

## What was actually verified end-to-end

Real, deployed, working — not simulated:

1. Pushed a container image to the deployed ECR repo, forced an ECS deployment, watched the Fargate task reach `RUNNING`.
2. NLB target health converged to `healthy` via the TCP stub health check on port 9002 (see §13.3) — confirmed both from `describe-target-health` and from `HEALTH_PORT` connection logs inside the container.
3. Sent 20 hand-encoded `ExecutionReport` UDP packets (using the exact `src/protocol.rs` wire format) from a local test client to the NLB's public DNS name, with a deliberate gap at `seq=10`.
4. Confirmed via CloudWatch Logs that the deployed subscriber decoded every packet correctly and logged `GAP detected — expected seq 10, got 11, missing 1 report(s)` at the right point — proving the exact §7.2 gap-detection logic works unmodified behind a real NLB + Fargate deployment.

## What wasn't tested (documented, not hidden)

- **No real multicast source.** The matching engine runs on the operator's own hardware per §11 and was not stood up as a live multicast source for this exercise. `examples/relay.rs` was built and reviewed but not run against a live multicast feed — the synthetic test client sent unicast UDP directly to the NLB, which exercises the same cloud-side path the relay would feed into, but doesn't exercise IGMP join behavior, LAN multicast loss, or the relay process itself under load.
- **A real deployment run hit a stuck ECS deployment state** during this exercise: three rapid successive `force-new-deployment` calls (used for iterating on the health-check fix) left the service with four overlapping deployment/task-set objects and a scheduler that stopped placing new tasks entirely — no events, no errors, just silence for over 10 minutes. The fix was deleting and recreating the `aws_ecs_service` resource via Terraform rather than trying to reconcile the stuck state. Documented because it's a real operational trap: **don't force-redeploy an ECS service repeatedly in quick succession while iterating** — let one rollout finish (or fail) before triggering the next.
- **NLB UDP target groups require a TCP (or HTTP/HTTPS) health check** — there is no way to health-check a bare UDP listener directly. This is documented in §13.3/§13.5 as a design decision (a stub TCP listener on a second port), but is worth restating here as a concrete thing that doesn't work the way the docs suggest it might: a UDP-only NLB target group with no accompanying TCP-capable health port simply never converges to healthy.
- **`terraform destroy` didn't fully tear down on the first pass.** `aws_ecr_repository` errored with `RepositoryNotEmptyException` — Terraform's ECR resource refuses to delete a repository that still holds images unless `force_delete = true` is set, and the initial `infra/ecr.tf` didn't set it (every other resource — NLB, ECS, IAM, security groups, log group — destroyed cleanly in the same run). Fixed by force-deleting the repository out-of-band, reconciling it out of Terraform state, and adding `force_delete = true` to the config so a future `destroy` doesn't need manual intervention. Small, but exactly the kind of thing that leaves orphaned billable resources behind if nobody checks that a destroy actually finished.
