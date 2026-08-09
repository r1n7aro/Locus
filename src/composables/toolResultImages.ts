import type { ImageAttachment } from "../types";

export const DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX = "locus-deferred-message:";

export function deferredSessionMessageImageId(image: ImageAttachment): string | null {
  if (!image.data.startsWith(DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX)) return null;
  return image.data.slice(DEFERRED_SESSION_MESSAGE_IMAGE_PREFIX.length).trim() || null;
}

export async function resolveToolResultImages(
  images: ImageAttachment[],
  loadMessageImages: (messageId: string) => Promise<ImageAttachment[]>,
): Promise<ImageAttachment[]> {
  const messageIds = Array.from(new Set(
    images
      .map(deferredSessionMessageImageId)
      .filter((messageId): messageId is string => !!messageId),
  ));
  if (messageIds.length === 0) return images;

  const loadedEntries = await Promise.all(messageIds.map(async (messageId) => (
    [messageId, await loadMessageImages(messageId)] as const
  )));
  const loadedByMessageId = new Map(loadedEntries);
  const emittedMessageIds = new Set<string>();

  return images.flatMap((image) => {
    const messageId = deferredSessionMessageImageId(image);
    if (!messageId) return [image];
    if (emittedMessageIds.has(messageId)) return [];
    emittedMessageIds.add(messageId);
    return loadedByMessageId.get(messageId) ?? [];
  });
}
