import { playCustomNotificationSound } from "./system";

export type NotificationSoundIntent =
  | "complete"
  | "subagentComplete"
  | "input"
  | "error"
  | "confirm";

export type NotificationSoundMode = "soft" | "bright" | "urgent";
export type NotificationSoundSource = "builtin" | "custom";

interface SoundProfile {
  frequencies: number[];
  duration: number;
  gap: number;
  volume: number;
  type: OscillatorType;
}

const soundProfiles: Record<NotificationSoundMode, Record<NotificationSoundIntent, SoundProfile>> = {
  soft: {
    complete: {
      frequencies: [660, 880],
      duration: 0.105,
      gap: 0.035,
      volume: 0.052,
      type: "sine",
    },
    subagentComplete: {
      frequencies: [520, 700],
      duration: 0.095,
      gap: 0.03,
      volume: 0.048,
      type: "sine",
    },
    input: {
      frequencies: [740, 740],
      duration: 0.085,
      gap: 0.055,
      volume: 0.058,
      type: "triangle",
    },
    error: {
      frequencies: [360, 280],
      duration: 0.12,
      gap: 0.045,
      volume: 0.056,
      type: "sawtooth",
    },
    confirm: {
      frequencies: [560, 760],
      duration: 0.095,
      gap: 0.04,
      volume: 0.052,
      type: "triangle",
    },
  },
  bright: {
    complete: {
      frequencies: [720, 960, 1240],
      duration: 0.105,
      gap: 0.035,
      volume: 0.088,
      type: "triangle",
    },
    subagentComplete: {
      frequencies: [620, 820, 1040],
      duration: 0.092,
      gap: 0.032,
      volume: 0.078,
      type: "triangle",
    },
    input: {
      frequencies: [880, 880, 1180],
      duration: 0.08,
      gap: 0.045,
      volume: 0.092,
      type: "triangle",
    },
    error: {
      frequencies: [440, 330, 250],
      duration: 0.115,
      gap: 0.04,
      volume: 0.088,
      type: "sawtooth",
    },
    confirm: {
      frequencies: [640, 860, 1120],
      duration: 0.09,
      gap: 0.035,
      volume: 0.084,
      type: "triangle",
    },
  },
  urgent: {
    complete: {
      frequencies: [760, 980, 1260],
      duration: 0.09,
      gap: 0.04,
      volume: 0.102,
      type: "square",
    },
    subagentComplete: {
      frequencies: [620, 820, 1040],
      duration: 0.08,
      gap: 0.04,
      volume: 0.092,
      type: "square",
    },
    input: {
      frequencies: [920, 920, 920, 1120],
      duration: 0.07,
      gap: 0.055,
      volume: 0.105,
      type: "square",
    },
    error: {
      frequencies: [420, 320, 250],
      duration: 0.11,
      gap: 0.035,
      volume: 0.104,
      type: "sawtooth",
    },
    confirm: {
      frequencies: [700, 700, 900],
      duration: 0.075,
      gap: 0.045,
      volume: 0.102,
      type: "square",
    },
  },
};

let audioContext: AudioContext | null = null;

const DEFAULT_SOUND_VOLUME = 50;
const SOUND_OUTPUT_GAIN = 1.35;

function normalizeSoundVolume(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_SOUND_VOLUME / 100;
  return Math.min(100, Math.max(0, value)) / 100;
}

function getAudioContext(): AudioContext | null {
  if (typeof window === "undefined") return null;
  if (audioContext?.state === "closed") {
    audioContext = null;
  }
  if (audioContext) return audioContext;
  if (!window.AudioContext) return null;

  audioContext = new window.AudioContext();
  return audioContext;
}

export async function playNotificationSound(
  intent: NotificationSoundIntent,
  mode: NotificationSoundMode = "bright",
  customFilePath = "",
  volume = DEFAULT_SOUND_VOLUME,
): Promise<void> {
  const volumeScale = normalizeSoundVolume(volume);
  if (volumeScale <= 0) return;
  const outputVolumeScale = volumeScale * SOUND_OUTPUT_GAIN;

  const trimmedCustomFilePath = customFilePath.trim();
  if (trimmedCustomFilePath) {
    const customPlayed = await playCustomNotificationSound(trimmedCustomFilePath, outputVolumeScale)
      .then(() => true)
      .catch(() => false);
    if (customPlayed) return;
  }

  const profile = soundProfiles[mode]?.[intent] ?? soundProfiles.soft[intent];
  const context = getAudioContext();
  if (!context) return;
  if (context.state === "suspended") {
    await context.resume();
  }
  if (context.state !== "running") return;

  const startedAt = context.currentTime + 0.01;
  profile.frequencies.forEach((frequency, index) => {
    const startAt = startedAt + index * (profile.duration + profile.gap);
    const endAt = startAt + profile.duration;
    const oscillator = context.createOscillator();
    const gain = context.createGain();

    oscillator.type = profile.type;
    oscillator.frequency.setValueAtTime(frequency, startAt);
    oscillator.frequency.exponentialRampToValueAtTime(
      Math.max(120, frequency * 0.88),
      endAt,
    );
    gain.gain.setValueAtTime(0.0001, startAt);
    gain.gain.exponentialRampToValueAtTime(
      profile.volume * outputVolumeScale,
      startAt + 0.012,
    );
    gain.gain.exponentialRampToValueAtTime(0.0001, endAt);

    oscillator.connect(gain);
    gain.connect(context.destination);
    oscillator.start(startAt);
    oscillator.stop(endAt + 0.02);
    oscillator.onended = () => {
      oscillator.disconnect();
      gain.disconnect();
    };
  });
}

export async function unlockNotificationSounds(): Promise<void> {
  const context = getAudioContext();
  if (!context) return;
  if (context.state === "suspended") {
    await context.resume();
  }
}
