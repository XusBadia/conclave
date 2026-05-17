// Curated list of clinical specialties shown as suggestions in the
// workspace-creation combobox. The stored value goes straight into the
// LLM prompt (see `crates/verdict/src/prompt.rs`), so we keep the labels
// human-readable and locale-aware. Users can still type free text — this
// list is a soft default, not a closed enum.

export interface Specialty {
  es: string;
  en: string;
}

export const SPECIALTIES: Specialty[] = [
  { es: "Cardiología", en: "Cardiology" },
  { es: "Cirugía general", en: "General surgery" },
  { es: "Cirugía colorrectal", en: "Colorectal surgery" },
  { es: "Cirugía cardiovascular", en: "Cardiovascular surgery" },
  { es: "Dermatología", en: "Dermatology" },
  { es: "Endocrinología", en: "Endocrinology" },
  { es: "Gastroenterología", en: "Gastroenterology" },
  { es: "Geriatría", en: "Geriatrics" },
  { es: "Ginecología y obstetricia", en: "Gynecology and obstetrics" },
  { es: "Hematología", en: "Hematology" },
  { es: "Infectología", en: "Infectious diseases" },
  { es: "Medicina familiar", en: "Family medicine" },
  { es: "Medicina interna", en: "Internal medicine" },
  { es: "Medicina intensiva", en: "Critical care medicine" },
  { es: "Medicina de urgencias", en: "Emergency medicine" },
  { es: "Nefrología", en: "Nephrology" },
  { es: "Neumología", en: "Pulmonology" },
  { es: "Neurología", en: "Neurology" },
  { es: "Neurocirugía", en: "Neurosurgery" },
  { es: "Oftalmología", en: "Ophthalmology" },
  { es: "Oncología médica", en: "Medical oncology" },
  { es: "Oncología radioterápica", en: "Radiation oncology" },
  { es: "Otorrinolaringología", en: "Otolaryngology" },
  { es: "Pediatría", en: "Pediatrics" },
  { es: "Psiquiatría", en: "Psychiatry" },
  { es: "Radiología", en: "Radiology" },
  { es: "Reumatología", en: "Rheumatology" },
  { es: "Traumatología y ortopedia", en: "Orthopedics" },
  { es: "Urología", en: "Urology" },
];

export function specialtyOptions(lang: string): string[] {
  const key: keyof Specialty = lang.startsWith("en") ? "en" : "es";
  return SPECIALTIES.map((s) => s[key]);
}
