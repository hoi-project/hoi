export interface Publication { schema: number; repository: string; site: string; downloads: string; contact: string; manifestKey: string; updaterKey: string }
export type PublicationLevel = 'site' | 'release';
export const PUBLICATION_LEVELS: Record<PublicationLevel, (keyof Publication)[]>;
export function publicationLevel(value?: string): PublicationLevel | undefined;
export function validatePublication(value: unknown, required?: boolean | PublicationLevel): Publication;
export function loadPublication(required?: boolean | PublicationLevel): Publication;
export function configureInstaller(source: string, config?: Publication): string;
