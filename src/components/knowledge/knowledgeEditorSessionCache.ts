import { DocumentSessionCache } from "../../document/documentSessionCache";

/** Compatibility adapter for knowledge-specific draft eviction policy. */
export class KnowledgeEditorSessionCache<T> extends DocumentSessionCache<T> {
  constructor(capacity = 24, canEvict: (value: T) => boolean = () => true) {
    super({ capacity, canEvict, protectLatest: true });
  }
}
