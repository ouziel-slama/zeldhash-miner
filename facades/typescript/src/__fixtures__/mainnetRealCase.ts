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
    "cHNidP8BAGACAAAAATf/KjoGIo8IPJcFnDircGdFEsKvQUm8tUVgrBZhrYEfAAAAAAD9////AgQVAAAAAAAAFgAU6p0gv7k4sqDXeKXY2Lwqr/dVw5UAAAAAAAAAAAVqA3pEIAAAAAAAAQEfcBcAAAAAAAAWABTqnSC/uTiyoNd4pdjYvCqv91XDlQAAAA==",
  expectedTxid: "e69e52f032732b21e97667daf37d4aa6218ac7952f70db89585f702fd7069fee",
  // OP_RETURN holds the nonce bytes in big-endian form.
  nonce: 0x7a4420n,
  opReturnHex: "7a4420",
};

