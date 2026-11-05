import { shallowReactive } from "vue";

// Branch listings double as the repository probe: a repository always lists its
// own checkout, so an empty listing means the checkout is outside Git. Unknown
// checkouts keep the picker until their first listing settles.
const repositories = shallowReactive(new Map<string, boolean>());

export function rememberWorktreeRepository(checkoutId: string, present: boolean): void {
  if (repositories.get(checkoutId) !== present) repositories.set(checkoutId, present);
}

export function worktreeRepositoryAbsent(checkoutId: string): boolean {
  return repositories.get(checkoutId) === false;
}

export function resetWorktreeRepositoriesForTests(): void {
  repositories.clear();
}
