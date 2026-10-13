/** Something long being done: its progress (0 to 1) and what it's on, as it goes. */
export type ActivityUpdate = (u: { value?: number; label?: string }) => void;
