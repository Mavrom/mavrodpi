export type ProtectionProfileId = "balanced" | "compatibility";

export interface ProtectionProfile {
  id: ProtectionProfileId;
  name: string;
  badge: string;
  description: string;
  args: string[];
}

export const PROTECTION_PROFILES: ProtectionProfile[] = [
  {
    id: "balanced",
    name: "Dengeli",
    badge: "ÖNERİLEN",
    description: "GoodbyeDPI -5; çoğu bağlantı için dengeli başlangıç profili.",
    args: ["-5"],
  },
  {
    id: "compatibility",
    name: "Uyumluluk",
    badge: "ALTERNATİF",
    description: "GoodbyeDPI -6; Dengeli profil sonuç vermediğinde denenebilir.",
    args: ["-6"],
  },
];

export const DEFAULT_PROFILE_ID: ProtectionProfileId = "balanced";

export function getProtectionProfile(id: ProtectionProfileId): ProtectionProfile {
  return (
    PROTECTION_PROFILES.find((profile) => profile.id === id) ??
    PROTECTION_PROFILES[0]
  );
}
