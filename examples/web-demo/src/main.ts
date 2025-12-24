import "./style.css";
import "./polyfills";

import { Psbt } from "bitcoinjs-lib";
import {
  TransactionBuilder,
  ZeldMiner,
  ZeldMinerError,
  ZeldMinerErrorCode,
} from "zeldhash-miner";
import type {
  MineResult,
  Network,
  ProgressStats,
  TxInput,
  TxOutput,
} from "zeldhash-miner";

type UtxoRow = {
  id: string;
  txid: string;
  vout: string;
  amount: string;
  scriptPubKey: string;
};

type OutputRow = {
  id: string;
  address: string;
  amount: string;
  mhinAmount: string;
  change: boolean;
};

type StatusTone = "ok" | "warn" | "error";

const TXID_REGEX = /^[0-9a-fA-F]{64}$/;
const HEX_REGEX = /^[0-9a-fA-F]+$/;
const NON_NEG_INTEGER_REGEX = /^\d+$/;

const dustLimitForAddress = (address: string): number => {
  const lower = address.toLowerCase();
  if (lower.startsWith("bc1p") || lower.startsWith("tb1p") || lower.startsWith("bcrt1p")) return 330;
  if (lower.startsWith("bc1q") || lower.startsWith("tb1q") || lower.startsWith("bcrt1q")) return 310;
  return 546; // conservative fallback for unexpected address formats
};

// Start inside the 4-byte nonce range so even long demo runs (tens of millions
// of attempts at higher targets) never cross a byte-length boundary. Crossing a
// boundary invalidates the prebuilt mining template and triggers WebGPU/CPU
// errors like “nonce range crosses byte-length boundary; split batch”.
const DEFAULT_START_NONCE = 0n;
const CPU_BATCH_SIZE = 256;
// Larger batches amortize WebGPU dispatch overhead; stays within 3-byte nonce
// length for the default start nonce and the demo’s search depth.
const GPU_BATCH_SIZE = 65_536;

const initialUtxo: UtxoRow = {
  id: crypto.randomUUID(),
  txid: "1f81ad6116ac6045b5bc4941afc212456770ab389c05973c088f22063a2aff37",
  vout: "0",
  amount: "6000",
  scriptPubKey: "0014ea9d20bfb938b2a0d778a5d8d8bc2aaff755c395",
};

const initialOutputs: OutputRow[] = [
  {
    id: crypto.randomUUID(),
    address: "bc1qa2wjp0ae8ze2p4mc5hvd30p24lm4tsu479mw0r",
    amount: "",
    mhinAmount: "",
    change: true,
  },
];

const state = {
  network: "mainnet" as Network,
  satsPerVbyte: 5,
  targetZeros: 6,
  useWebGPU: true,
  gpuFallbackUsed: false,
  utxos: [initialUtxo],
  outputs: initialOutputs,
  mining: false,
  miner: null as ZeldMiner | null,
  abortController: null as AbortController | null,
  hashRateHistory: [] as number[],
  attempts: 0n,
  elapsed: 0,
  feeEstimate: null as number | null,
  result: null as MineResult | null,
  statusText: "Ready to mine",
  statusTone: "ok" as StatusTone,
};

const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("App container missing");
}

app.innerHTML = `
  <header class="header">
    <div class="title">
      <span class="dot"></span>
      <span>Zeldhash Miner Demo</span>
    </div>
    <div class="badge" id="networkBadge">Mainnet</div>
  </header>

  <div class="grid">
    <section class="panel">
      <h2>Network & Fees</h2>
      <div class="section">
        <div class="subgrid">
          <div class="field">
            <label for="networkSelect">Network</label>
            <select id="networkSelect">
              <option value="mainnet" selected>Mainnet</option>
              <option value="testnet">Testnet</option>
              <option value="signet">Signet</option>
            </select>
            <span class="muted">Affects address validation and fee policies.</span>
          </div>
          <div class="field">
            <label for="feeInput">Fee rate (sats/vB)</label>
            <input id="feeInput" type="number" min="1" step="1" value="5" />
            <span class="muted">Used to size the transaction when building the mining template.</span>
          </div>
        </div>
        <div class="row">
          <label class="chip">
            <input id="webGpuToggle" type="checkbox" checked />
            Use WebGPU (fall back to CPU if unavailable)
          </label>
          <button class="button secondary" id="estimateFeeBtn" type="button">Estimate fee</button>
          <span class="muted" id="feeEstimateText">No estimate yet.</span>
        </div>
      </div>
    </section>

    <section class="panel">
      <h2>UTXO Inputs</h2>
      <div id="utxoList" class="list"></div>
      <div class="button-row">
        <button class="button secondary" id="addUtxoBtn" type="button">Add input</button>
      </div>
    </section>

    <section class="panel">
      <h2>Outputs</h2>
      <div id="outputList" class="list"></div>
      <div class="button-row">
        <button class="button secondary" id="addOutputBtn" type="button">Add output</button>
        <span class="muted">Mark exactly one output as change. Others need an explicit amount.</span>
      </div>
    </section>

    <section class="panel">
      <h2>Mining Dashboard</h2>
      <div class="section">
        <div class="field">
          <label for="targetSlider">Target zeros: <strong id="targetValue">6</strong></label>
          <input id="targetSlider" class="slider" type="range" min="1" max="8" value="6" />
          <span class="muted">Lower targets are faster to find; higher targets take longer.</span>
        </div>

        <div class="miner-visual" id="minerVisual">
          <div class="orb"></div>
          <div class="row">
            <div class="chip">COOP/COEP ready</div>
            <div class="chip" id="modeChip">GPU preferred</div>
          </div>
        </div>

        <div class="stat-cards">
          <div class="stat">
            <label>Hash rate</label>
            <div class="value" id="hashRateValue">0 H/s</div>
          </div>
          <div class="stat">
            <label>Attempts</label>
            <div class="value" id="attemptValue">0</div>
          </div>
          <div class="stat">
            <label>Elapsed</label>
            <div class="value" id="elapsedValue">0.0s</div>
          </div>
        </div>

        <div class="field">
          <label>Hash rate visualization</label>
          <div class="hash-bars" id="hashBars"></div>
        </div>

        <div class="field">
          <label>Progress pulse</label>
          <div class="progress-bar">
            <div class="progress-fill" id="progressFill"></div>
          </div>
        </div>

        <div class="button-row">
          <button class="button" id="startBtn" type="button">Start mining</button>
          <button class="button secondary" id="stopBtn" type="button" disabled>Stop</button>
        </div>

        <div class="status" id="statusBox">
          <span class="dot"></span>
          <span id="statusText">Ready to mine</span>
        </div>
      </div>
    </section>

    <section class="panel">
      <h2>Result</h2>
      <div class="section">
        <div class="pill-group">
          <span class="pill" id="resultTxid">No result yet</span>
          <span class="pill" id="resultNonce"></span>
          <span class="pill" id="resultDuration"></span>
        </div>
        <div class="field">
          <label>PSBT</label>
          <div class="psbt" id="psbtValue">Awaiting mining result…</div>
        </div>
        <div class="actions">
          <button class="button secondary" id="copyPsbtBtn" type="button" disabled>Copy PSBT</button>
          <button class="button secondary" id="downloadPsbtBtn" type="button" disabled>Download PSBT</button>
        </div>
      </div>
    </section>
  </div>
`;

const ui = {
  networkSelect: document.querySelector<HTMLSelectElement>("#networkSelect")!,
  networkBadge: document.querySelector<HTMLDivElement>("#networkBadge")!,
  feeInput: document.querySelector<HTMLInputElement>("#feeInput")!,
  estimateFeeBtn: document.querySelector<HTMLButtonElement>("#estimateFeeBtn")!,
  feeEstimateText: document.querySelector<HTMLSpanElement>("#feeEstimateText")!,
  webGpuToggle: document.querySelector<HTMLInputElement>("#webGpuToggle")!,
  utxoList: document.querySelector<HTMLDivElement>("#utxoList")!,
  addUtxoBtn: document.querySelector<HTMLButtonElement>("#addUtxoBtn")!,
  outputList: document.querySelector<HTMLDivElement>("#outputList")!,
  addOutputBtn: document.querySelector<HTMLButtonElement>("#addOutputBtn")!,
  targetSlider: document.querySelector<HTMLInputElement>("#targetSlider")!,
  targetValue: document.querySelector<HTMLSpanElement>("#targetValue")!,
  hashRateValue: document.querySelector<HTMLDivElement>("#hashRateValue")!,
  attemptValue: document.querySelector<HTMLDivElement>("#attemptValue")!,
  elapsedValue: document.querySelector<HTMLDivElement>("#elapsedValue")!,
  hashBars: document.querySelector<HTMLDivElement>("#hashBars")!,
  progressFill: document.querySelector<HTMLDivElement>("#progressFill")!,
  startBtn: document.querySelector<HTMLButtonElement>("#startBtn")!,
  stopBtn: document.querySelector<HTMLButtonElement>("#stopBtn")!,
  statusBox: document.querySelector<HTMLDivElement>("#statusBox")!,
  statusText: document.querySelector<HTMLSpanElement>("#statusText")!,
  minerVisual: document.querySelector<HTMLDivElement>("#minerVisual")!,
  modeChip: document.querySelector<HTMLDivElement>("#modeChip")!,
  resultTxid: document.querySelector<HTMLSpanElement>("#resultTxid")!,
  resultNonce: document.querySelector<HTMLSpanElement>("#resultNonce")!,
  resultDuration: document.querySelector<HTMLSpanElement>("#resultDuration")!,
  psbtValue: document.querySelector<HTMLDivElement>("#psbtValue")!,
  copyPsbtBtn: document.querySelector<HTMLButtonElement>("#copyPsbtBtn")!,
  downloadPsbtBtn: document.querySelector<HTMLButtonElement>("#downloadPsbtBtn")!,
  confetti: document.querySelector<HTMLDivElement>("#confetti")!,
};

const formatHashRate = (rate: number): string => {
  if (rate >= 1_000_000_000) return `${(rate / 1_000_000_000).toFixed(2)} GH/s`;
  if (rate >= 1_000_000) return `${(rate / 1_000_000).toFixed(2)} MH/s`;
  if (rate >= 1_000) return `${(rate / 1_000).toFixed(2)} kH/s`;
  return `${rate.toFixed(2)} H/s`;
};

const formatDuration = (ms: number): string => `${(ms / 1000).toFixed(2)}s`;

const formatBig = (value: bigint): string =>
  value.toLocaleString("en-US", { maximumFractionDigits: 0 });

const setStatus = (message: string, tone: StatusTone = "ok"): void => {
  state.statusText = message;
  state.statusTone = tone;
  ui.statusText.textContent = message;
  ui.statusBox.classList.remove("warning", "danger");
  if (tone === "warn") ui.statusBox.classList.add("warning");
  if (tone === "error") ui.statusBox.classList.add("danger");
};

const updateNetworkBadge = (): void => {
  const label = state.network === "mainnet" ? "Mainnet" : state.network === "signet" ? "Signet" : "Testnet";
  ui.networkBadge.textContent = label;
};

const renderHashBars = (): void => {
  const history = state.hashRateHistory;
  const max = Math.max(...history, 1);
  ui.hashBars.innerHTML = "";
  history.slice(-40).forEach((rate) => {
    const bar = document.createElement("div");
    bar.className = "hash-bar";
    const strength = rate / max;
    if (strength < 0.33) bar.classList.add("low");
    else if (strength < 0.66) bar.classList.add("mid");
    else bar.classList.add("high");

    const height = Math.max(8, Math.min(100, strength * 100));
    bar.style.height = `${height}%`;
    ui.hashBars.appendChild(bar);
  });
};

const renderUtxos = (): void => {
  ui.utxoList.innerHTML = "";
  state.utxos.forEach((utxo, index) => {
    const wrapper = document.createElement("div");
    wrapper.className = "list-item";

    const header = document.createElement("div");
    header.className = "list-item-header";
    header.innerHTML = `<span>Input #${index + 1}</span>`;

    const removeBtn = document.createElement("button");
    removeBtn.className = "button secondary";
    removeBtn.type = "button";
    removeBtn.textContent = "Remove";
    removeBtn.disabled = state.utxos.length === 1;
    removeBtn.addEventListener("click", () => {
      state.utxos = state.utxos.filter((item) => item.id !== utxo.id);
      renderUtxos();
    });
    header.appendChild(removeBtn);

    const grid = document.createElement("div");
    grid.className = "subgrid";
    grid.innerHTML = `
      <div class="field">
        <label>Txid</label>
        <input data-field="txid" value="${utxo.txid}" placeholder="64-char hex" />
      </div>
      <div class="field">
        <label>Vout</label>
        <input data-field="vout" type="number" min="0" value="${utxo.vout}" />
      </div>
      <div class="field">
        <label>Amount (sats)</label>
        <input data-field="amount" type="number" min="1" value="${utxo.amount}" />
      </div>
      <div class="field">
        <label>scriptPubKey (hex)</label>
        <input data-field="scriptPubKey" value="${utxo.scriptPubKey}" placeholder="0014..." />
      </div>
    `;

    grid.querySelectorAll<HTMLInputElement>("input").forEach((input) => {
      input.addEventListener("input", () => {
        const field = input.dataset.field as keyof UtxoRow;
        state.utxos = state.utxos.map((item) =>
          item.id === utxo.id ? { ...item, [field]: input.value } : item
        );
      });
    });

    wrapper.appendChild(header);
    wrapper.appendChild(grid);
    ui.utxoList.appendChild(wrapper);
  });
};

const renderOutputs = (): void => {
  ui.outputList.innerHTML = "";
  const changeSelected = state.outputs.find((o) => o.change)?.id;

  state.outputs.forEach((output, index) => {
    const wrapper = document.createElement("div");
    wrapper.className = "list-item";

    const header = document.createElement("div");
    header.className = "list-item-header";
    header.innerHTML = `<span>Output #${index + 1}</span>`;

    const removeBtn = document.createElement("button");
    removeBtn.className = "button secondary";
    removeBtn.type = "button";
    removeBtn.textContent = "Remove";
    removeBtn.disabled = state.outputs.length === 1;
    removeBtn.addEventListener("click", () => {
      state.outputs = state.outputs.filter((item) => item.id !== output.id);
      if (!state.outputs.some((o) => o.change)) {
        const first = state.outputs[0];
        if (first) state.outputs[0] = { ...first, change: true };
      }
      renderOutputs();
    });
    header.appendChild(removeBtn);

    const grid = document.createElement("div");
    grid.className = "subgrid";
    grid.innerHTML = `
      <div class="field">
        <label>Address</label>
        <input data-field="address" value="${output.address}" placeholder="bc1... or tb1..." />
      </div>
      <div class="field">
        <label>Amount (sats)</label>
        <input data-field="amount" type="number" min="0" value="${output.amount}" ${output.change ? "placeholder='Auto for change'" : ""}/>
        <span class="muted">${output.change ? "Leave empty to auto-calc change" : "Required for non-change outputs"}</span>
      </div>
      <div class="field">
        <label>MHIN Amount (sats)</label>
        <input data-field="mhinAmount" type="number" min="0" value="${output.mhinAmount}" placeholder="Optional" />
        <span class="muted">Optional ZELD distribution value; blanks become 0 if any are set.</span>
      </div>
      <div class="field">
        <label class="tag ${output.change ? "positive" : ""}">
          <input data-field="change" type="radio" name="changeOutput" ${output.change ? "checked" : ""}/>
          Change output
        </label>
      </div>
    `;

    grid.querySelectorAll<HTMLInputElement>("input").forEach((input) => {
      const field = input.dataset.field as keyof OutputRow;
      if (field === "change") {
        input.addEventListener("change", () => {
          state.outputs = state.outputs.map((item) => ({
            ...item,
            change: item.id === output.id,
          }));
          renderOutputs();
        });
      } else {
        input.addEventListener("input", () => {
          state.outputs = state.outputs.map((item) =>
            item.id === output.id ? { ...item, [field]: input.value } : item
          );
        });
      }
    });

    wrapper.appendChild(header);
    wrapper.appendChild(grid);
    ui.outputList.appendChild(wrapper);
  });
};

const parseUtxos = (): TxInput[] => {
  if (!state.utxos.length) throw new Error("Add at least one input.");
  return state.utxos.map((row, idx) => {
    const txid = row.txid.trim();
    const scriptPubKey = row.scriptPubKey.trim();

    if (
      !txid ||
      row.vout === "" ||
      row.amount === "" ||
      !scriptPubKey
    ) {
      throw new Error(`Input #${idx + 1} is incomplete.`);
    }
    if (!TXID_REGEX.test(txid)) {
      throw new Error(`Input #${idx + 1} txid must be 64-char hex.`);
    }
    const vout = Number(row.vout);
    const amount = Number(row.amount);
    if (!Number.isFinite(vout) || vout < 0) {
      throw new Error(`Input #${idx + 1} has an invalid vout.`);
    }
    if (!Number.isFinite(amount) || amount <= 0) {
      throw new Error(`Input #${idx + 1} has an invalid amount.`);
    }
    if (!HEX_REGEX.test(scriptPubKey) || scriptPubKey.length % 2 !== 0) {
      throw new Error(`Input #${idx + 1} scriptPubKey must be valid hex.`);
    }
    return {
      txid,
      vout,
      amount,
      scriptPubKey,
    };
  });
};

const parseOutputs = (): TxOutput[] => {
  if (!state.outputs.length) throw new Error("Add at least one output.");
  const changeId = state.outputs.find((o) => o.change)?.id;
  if (!changeId) throw new Error("Mark one output as change.");

  return state.outputs.map((row, idx) => {
    if (!row.address.trim()) throw new Error(`Output #${idx + 1} needs an address.`);
    const amountRaw = row.amount.trim();
    const parsedAmount = Number(amountRaw);
    const amountNum =
      amountRaw === "" ? undefined : Number.isFinite(parsedAmount) ? parsedAmount : undefined;
    const dustLimit = dustLimitForAddress(row.address.trim());

    if (!row.change && (amountNum === undefined || amountNum < dustLimit)) {
      throw new Error(`Output #${idx + 1} must be at least ${dustLimit} sats.`);
    }
    if (row.change && amountNum !== undefined && amountNum < 0) {
      throw new Error(`Change output #${idx + 1} cannot be negative.`);
    }
    return {
      address: row.address.trim(),
      amount: amountNum,
      change: row.id === changeId,
    };
  });
};

const buildDistribution = (outputs: OutputRow[]): bigint[] | undefined => {
  const parsed = outputs.map((row, idx) => {
    const raw = row.mhinAmount.trim();
    if (raw === "") return null;
    if (!NON_NEG_INTEGER_REGEX.test(raw)) {
      throw new Error(`Output #${idx + 1} MHIN amount must be a non-negative integer.`);
    }
    return BigInt(raw);
  });

  if (!parsed.some((value) => value !== null)) return undefined;

  return parsed.map((value) => value ?? 0n);
};

const updateResultPanel = (result: MineResult | null): void => {
  if (!result) {
    ui.resultTxid.textContent = "No result yet";
    ui.resultNonce.textContent = "";
    ui.resultDuration.textContent = "";
    ui.psbtValue.textContent = "Awaiting mining result…";
    ui.copyPsbtBtn.disabled = true;
    ui.downloadPsbtBtn.disabled = true;
    return;
  }

  ui.resultTxid.textContent = `txid: ${result.txid}`;
  ui.resultNonce.textContent = `nonce: ${result.nonce.toString()}`;
  ui.resultDuration.textContent = `duration: ${formatDuration(result.duration)}`;
  ui.psbtValue.textContent = result.psbt;
  ui.copyPsbtBtn.disabled = false;
  ui.downloadPsbtBtn.disabled = false;
};

const updateMiningUi = (active: boolean): void => {
  state.mining = active;
  ui.startBtn.disabled = active;
  ui.stopBtn.disabled = !active;
  ui.minerVisual.classList.toggle("active", active);
  ui.progressFill.style.width = active ? "15%" : "0%";
};

const updateProgress = (stats: ProgressStats): void => {
  state.hashRateHistory = [...state.hashRateHistory.slice(-39), stats.hashRate];
  state.attempts = stats.hashesProcessed;
  state.elapsed = stats.elapsedMs ?? 0;

  ui.hashRateValue.textContent = formatHashRate(stats.hashRate);
  ui.attemptValue.textContent = formatBig(stats.hashesProcessed);
  ui.elapsedValue.textContent = formatDuration(state.elapsed);

  renderHashBars();

  const mod = Number(stats.hashesProcessed % 100000n);
  const pct = Math.max(10, (mod / 100000) * 100);
  ui.progressFill.style.width = `${pct}%`;
};

const resetProgress = (): void => {
  state.hashRateHistory = [];
  state.attempts = 0n;
  state.elapsed = 0;
  renderHashBars();
  ui.hashRateValue.textContent = "0 H/s";
  ui.attemptValue.textContent = "0";
  ui.elapsedValue.textContent = "0.0s";
  ui.progressFill.style.width = "0%";
};

const triggerConfetti = (): void => {
  ui.confetti.classList.remove("active");
  // Restart animation
  void ui.confetti.offsetWidth;
  ui.confetti.classList.add("active");
};

const estimateFee = async (): Promise<void> => {
  setStatus("Estimating fee…");
  try {
    const inputs = parseUtxos();
    const outputs = parseOutputs();
    const distribution = buildDistribution(state.outputs);
    const builder = new TransactionBuilder(state.network, state.satsPerVbyte);
    const psbtBase64 = await builder.buildPsbt({
      inputs,
      outputs,
      nonce: 0n,
      distribution,
    });

    const psbt = Psbt.fromBase64(psbtBase64);
    const outputTotal = psbt.txOutputs.reduce((sum, o) => sum + o.value, 0);
    const inputTotal = inputs.reduce((sum, i) => sum + i.amount, 0);
    const fee = inputTotal - outputTotal;

    state.feeEstimate = fee;
    ui.feeEstimateText.textContent = `Estimated fee: ${fee.toLocaleString()} sats`;
    setStatus("Fee estimation succeeded");
  } catch (err) {
    ui.feeEstimateText.textContent = "Could not estimate fee";
    const message = err instanceof Error ? err.message : String(err);
    setStatus(`Fee estimation failed: ${message}`, "warn");
  }
};

const handleMiningSuccess = (result: MineResult): void => {
  state.result = result;
  updateResultPanel(result);
  setStatus("Found valid hash! PSBT ready.", "ok");
  triggerConfetti();
  updateMiningUi(false);
};

const handleMiningError = (err: unknown): void => {
  if (err instanceof ZeldMinerError) {
    if (
      err.code === ZeldMinerErrorCode.WEBGPU_NOT_AVAILABLE &&
      state.useWebGPU &&
      !state.gpuFallbackUsed
    ) {
      state.gpuFallbackUsed = true;
      state.useWebGPU = false;
      ui.webGpuToggle.checked = false;
      ui.modeChip.textContent = "CPU mode";
      setStatus(
        "WebGPU is unavailable in this browser; falling back to CPU…",
        "warn"
      );
      updateMiningUi(false);
      void startMining();
      return;
    }
    setStatus(`Mining error: ${err.code}: ${err.message}`, "error");
  } else if (err instanceof Error) {
    setStatus(`Mining error: ${err.message}`, "error");
  } else {
    setStatus(`Mining error: ${String(err)}`, "error");
  }
  updateMiningUi(false);
};

const startMining = async (): Promise<void> => {
  if (state.mining) return;
  try {
    const inputs = parseUtxos();
    const outputs = parseOutputs();
    const distribution = buildDistribution(state.outputs);

    state.satsPerVbyte = Math.max(1, Number(ui.feeInput.value) || 1);
    state.targetZeros = Number(ui.targetSlider.value);
    state.network = ui.networkSelect.value as Network;
    state.useWebGPU = ui.webGpuToggle.checked;

    const batchSize = state.useWebGPU ? GPU_BATCH_SIZE : CPU_BATCH_SIZE;

    state.miner = new ZeldMiner({
      network: state.network,
      batchSize,
      useWebGPU: state.useWebGPU,
      workerThreads: Math.max(1, navigator.hardwareConcurrency || 4),
      satsPerVbyte: state.satsPerVbyte,
    });

    const miner = state.miner;
    const abort = new AbortController();
    state.abortController = abort;
    updateMiningUi(true);
    resetProgress();
    updateNetworkBadge();
    ui.modeChip.textContent = state.useWebGPU ? "GPU preferred" : "CPU mode";
    setStatus("Building template and starting workers…");

    miner.on("progress", updateProgress);
    miner.on("found", handleMiningSuccess);
    miner.on("error", handleMiningError);
    miner.on("stopped", () => {
      if (!state.mining) return;
      setStatus("Mining stopped", "warn");
      updateMiningUi(false);
    });

    void miner
      .mineTransaction({
        inputs,
        outputs,
        targetZeros: state.targetZeros,
        startNonce: DEFAULT_START_NONCE,
        signal: abort.signal,
        distribution,
      })
      .catch(handleMiningError);

    setStatus("Mining… keep this tab focused for best performance.");
  } catch (err) {
    handleMiningError(err);
  }
};

const stopMining = (): void => {
  if (!state.mining) return;
  state.abortController?.abort();
  state.miner?.stop();
  updateMiningUi(false);
  setStatus("Mining stopped by user", "warn");
};

const copyPsbt = async (): Promise<void> => {
  if (!state.result?.psbt) return;
  try {
    await navigator.clipboard.writeText(state.result.psbt);
    setStatus("PSBT copied to clipboard");
  } catch {
    setStatus("Clipboard copy failed", "warn");
  }
};

const downloadPsbt = (): void => {
  if (!state.result?.psbt) return;
  const blob = new Blob([state.result.psbt], { type: "text/plain" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = `zeldhash-${state.result.txid}.psbt`;
  link.click();
  URL.revokeObjectURL(url);
};

// Wire up events
ui.networkSelect.addEventListener("change", () => {
  state.network = ui.networkSelect.value as Network;
  updateNetworkBadge();
});

ui.feeInput.addEventListener("change", () => {
  const value = Number(ui.feeInput.value);
  state.satsPerVbyte = Math.max(1, Number.isFinite(value) ? value : 1);
});

ui.webGpuToggle.addEventListener("change", () => {
  state.useWebGPU = ui.webGpuToggle.checked;
  state.gpuFallbackUsed = false;
  ui.modeChip.textContent = state.useWebGPU ? "GPU preferred" : "CPU mode";
});

ui.targetSlider.addEventListener("input", () => {
  state.targetZeros = Number(ui.targetSlider.value);
  ui.targetValue.textContent = state.targetZeros.toString();
});

ui.addUtxoBtn.addEventListener("click", () => {
  state.utxos = [
    ...state.utxos,
    { id: crypto.randomUUID(), txid: "", vout: "", amount: "", scriptPubKey: "" },
  ];
  renderUtxos();
});

ui.addOutputBtn.addEventListener("click", () => {
  state.outputs = [
    ...state.outputs,
    { id: crypto.randomUUID(), address: "", amount: "", mhinAmount: "", change: false },
  ];
  renderOutputs();
});

ui.estimateFeeBtn.addEventListener("click", () => void estimateFee());
ui.startBtn.addEventListener("click", () => void startMining());
ui.stopBtn.addEventListener("click", stopMining);
ui.copyPsbtBtn.addEventListener("click", () => void copyPsbt());
ui.downloadPsbtBtn.addEventListener("click", downloadPsbt);

renderUtxos();
renderOutputs();
renderHashBars();
updateNetworkBadge();
updateResultPanel(null);
setStatus("Ready to mine");

