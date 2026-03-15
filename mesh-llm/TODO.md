# mesh-llm TODO

## Connection Stability — Studio-specific

**Studio is the problem, not the relay.** 5-minute drop count comparison on dedicated iroh relay:
- Local: **1 drop** (and that 1 was to Studio)
- Mini: **15 drops** (all to Studio)
- Studio: **100 drops**

Mini↔Fly nodes are rock solid (0 failures across all heartbeats). Local↔Fly and Local↔Mini also 100% stable. Only connections involving Studio flap. Even Local→Studio on the same LAN (10ms direct path) times out periodically.

Dedicated iroh relay (`usw1-2.relay.michaelneale.mesh-llm.iroh.link`) works great for everyone except Studio. Switched from old Fly relay in v0.35.2.

Next steps:
- [ ] Check Studio's macOS firewall settings — is it interfering with iroh's UDP/QUIC?
- [ ] Check Studio's sleep/power settings — is the network interface sleeping?
- [ ] Test Studio with a fresh identity (delete `~/.mesh-llm/key`)
- [ ] Run `iroh doctor` or similar diagnostics on Studio vs Mini
- [ ] Check if Studio has any network extensions, VPN, or Little Snitch
- [ ] Profile iroh endpoint on Studio — is it the QUIC layer or the relay websocket?

## Mac Native App

Simple native macOS app (SwiftUI) that starts or joins a mesh.

- Menubar app or lightweight window
- If machine has large VRAM (≥24GB): offer to **start** a mesh (runs mesh-llm --auto, shows invite QR)
- If machine has small VRAM: offer to **join** a mesh (paste/scan invite token, client-only)
- Shows mesh status: peers, models, throughput
- Bundles mesh-llm binary, manages lifecycle (start/stop/restart)
- First-class macOS citizen: signed, notarized, drag-to-Applications

## Mobile Chat App (exemplar)

Build a delightful mobile chat app that connects to any mesh as a client-only node.

- Scan a QR code (mesh invite token) to join
- Client-only: no GPU, no model serving — just routes inference through mesh hosts
- Uses iroh relay for connectivity (works through NAT, cellular, etc.)
- Minimal native UI — conversation list, chat bubbles, model picker from mesh catalog
- Target: iOS first (Swift + iroh-ffi), Android follow-up
- Shows off mesh-llm's value: scan a code, get access to a GPU pool, no setup
- Think "AirDrop for AI" — one scan and you're chatting with a 235B model
- OpenAI-compatible API underneath, so could also power shortcuts/widgets

## Smart Router
- [x] Heuristic classifier: Code/Reasoning/Chat/Creative/ToolCall categories
- [x] Complexity detection: Quick/Moderate/Deep from message signals
- [x] Task-dominant scoring: match bonus + tier + position
- [x] Tool capability filter: hard gate on `tools: bool` per model profile
- [x] needs_tools as attribute, not category override
- [ ] **Static speed estimates**: Add `tok_s: f64` to ModelProfile (known from benchmarks, no runtime measurement). Feed into scoring so Quick tasks prefer fast models.
- [ ] **Response quality checks**: Detect empty/repetitive/truncated responses, trigger retry with different model. Needs proxy to inspect response bytes (currently raw TCP relay).
- [ ] **Complexity → context budget**: Deep requests get larger `-n` (max tokens), Quick gets smaller. Currently all requests use llama-server defaults.

## Multi-Model Serving
- [x] `--model A --model B` runs separate election loops per model
- [x] Auto model packs by VRAM tier
- [x] `serving_models: Vec<String>` in gossip (backward compatible)
- [x] Router picks best model per request
- [ ] **Demand-based model upgrade**: Large-VRAM host serving a small model should upgrade when demand exists for a bigger model nobody is serving.

## First-Time Experience
- [ ] **Solo fallback — fast starter model**: When `--auto` finds no mesh, download a small starter model first (Qwen2.5-3B, 2GB, ~1 min), start serving immediately, then background-download a better model for the node's VRAM tier.
- [ ] **Uptime signal**: Add `started_at: u64` to `MeshListing`. Score bonus for longer-running meshes.

## Model Catalog
- [ ] **Draft model completeness**: GLM-4.7 and DeepSeek have no draft pairing.
- [ ] **Don't download what won't fit**: Check VRAM before downloading via `--model`.
- [ ] `mesh-llm recommend`: CLI subcommand to suggest models for your hardware.

## MoE Expert Sharding

Design: [MoE_PLAN.md](../MoE_PLAN.md) · Auto-deploy: [MoE_DEPLOY_DESIGN.md](../MoE_DEPLOY_DESIGN.md) · Validation: [MoE_SPLIT_REPORT.md](../MoE_SPLIT_REPORT.md)

- [x] Phase 1–3: Routing analysis, expert masking, mesh integration. Tested OLMoE-1B-7B over WAN.
- [ ] **Phase 4: lazy `moe-analyze`** — auto-run ranking for unknown MoE models. Currently unknown models fall through to PP.
- [ ] **Phase 5: probe-based session placement** — parked on `moe-probe` branch.
- [ ] **Phase 6: scale testing** — Mixtral 8×22B, Qwen3-235B-A22B.

## Resilience
- [x] Nostr re-discovery on peer loss
- [x] llama-server death watchdog
- [x] Multi-host load balancing
- [x] Demand-based duplicate hosting
- [x] API deadlock fix (v0.35.1) — snapshot locks independently, never hold multiple
- [x] VRAM-scaled context sizes — prevents OOM on small machines
- [ ] **Multi-node tensor split recovery**: If one split peer dies, re-split across remaining.
- [ ] **`kill_llama_server()` uses `pkill -f`**: Should kill by PID, not pattern match.

## Discovery & Publishing
- [ ] **Revisit `--publish` flag**: Bare `--publish` without `--mesh-name` is vestigial.

## Experiments
- [ ] Qwen3.5-397B-A17B across 128GB M4 Max + second machine (MoE, ~219GB Q4)
- [ ] Largest dense models across 2+ machines (Llama-3.3-70B, Qwen2.5-72B)
- [ ] MiniMax-M2.5 MoE split across Studio + second large machine
