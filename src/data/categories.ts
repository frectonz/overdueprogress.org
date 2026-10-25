import essaysJson from "./essays.json";

export type Essay = {
  id: number;
  title: string;
  author: string;
  link: string;
  category: string;
};

export const CATEGORIES: { slug: string; title: string }[] = [
  { slug: "money", title: "Money, Currency & Financial Systems" },
  { slug: "education", title: "Education & Human Capital" },
  { slug: "agriculture", title: "Agriculture, Food & Supply Chains" },
  { slug: "health", title: "Health" },
  { slug: "cities", title: "Cities, Housing & Transport" },
  { slug: "energy", title: "Energy, Technology & Digital Life" },
  { slug: "state", title: "State, Institutions & Governance" },
  { slug: "society", title: "History, Culture & Society" },
  { slug: "africa", title: "Africa & Continental Progress" },
  { slug: "personal", title: "Personal Essays & Reflections" },
];

export const essays: Essay[] = essaysJson;

export function essaysIn(slug: string): Essay[] {
  return essays.filter((e) => e.category === slug);
}
