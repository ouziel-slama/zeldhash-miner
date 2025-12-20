import type { Network, TxInput, TxOutput } from "../types";

export const MAINNET_REAL_CASE = {
  network: "mainnet" as Network,
  satsPerVbyte: 5,
  targetZeros: 6,
  inputs: [
    {
      txid: "1f81ad6116ac6045b5bc4941afc212456770ab389c05973c088f22063a2aff37",
      vout: 0,
      amount: 6000,
      scriptPubKey: "0014ea9d20bfb938b2a0d778a5d8d8bc2aaff755c395",
    },
  ] as TxInput[],
  outputs: [
    {
      address: "bc1qa2wjp0ae8ze2p4mc5hvd30p24lm4tsu479mw0r",
      change: true,
    },
  ] as TxOutput[],
  expectedPsbt:
    "cHNidP8BAGACAAAAATf/KjoGIo8IPJcFnDircGdFEsKvQUm8tUVgrBZhrYEfAAAAAAD9////AgAAAAAAAAAABWoDIER6BBUAAAAAAAAWABTqnSC/uTiyoNd4pdjYvCqv91XDlQAAAAAAAQEfcBcAAAAAAAAWABTqnSC/uTiyoNd4pdjYvCqv91XDlQAAAA==",
  expectedTxid: "000000dc1c2ae6c5bb67b149645a140e5c0c78c812e2c734feef21eb4abdabe0",
  // OP_RETURN holds the nonce bytes in little-endian form.
  nonce: 0x7a4420n,
  opReturnHex: "20447a",
};

