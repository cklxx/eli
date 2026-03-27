import type { EliBridgeMediaItem, EliChannelMessage } from "./types.js";

export const ELI_BRIDGE_CONTRACT_VERSION = "eli.sidecar.v1";

export function withBridgeContract(
  message: Omit<EliChannelMessage, "contract_version">,
): EliChannelMessage {
  return { ...message, contract_version: ELI_BRIDGE_CONTRACT_VERSION };
}

export function resolveBridgeContractVersion(version: unknown): string {
  if (version === undefined || version === null || version === "") {
    return ELI_BRIDGE_CONTRACT_VERSION;
  }
  if (version === ELI_BRIDGE_CONTRACT_VERSION) {
    return version;
  }
  throw new Error(`unsupported contract_version: ${String(version)}`);
}

export function outboundMediaItems(msg: EliChannelMessage): EliBridgeMediaItem[] {
  if (Array.isArray(msg.media) && msg.media.length > 0) {
    return msg.media;
  }
  return Array.isArray(msg.context?.outbound_media) ? msg.context.outbound_media : [];
}
