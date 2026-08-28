export type NotificationType = "success" | "error" | "warning" | "info";

export interface Notification {
  id: string;
  type: NotificationType;
  title?: string;
  message: string;
  /** Auto-dismiss after this many ms. 0 = persistent. Default: 5000 */
  duration: number;
}

class NotificationStore {
  items = $state<Notification[]>([]);

  add(notification: Omit<Notification, "id" | "duration"> & { duration?: number }): string {
    const id = crypto.randomUUID();
    const n: Notification = { ...notification, id, duration: notification.duration ?? 5000 };
    this.items = [...this.items, n];
    if (n.duration > 0) {
      setTimeout(() => this.dismiss(id), n.duration);
    }
    return id;
  }

  dismiss(id: string): void {
    this.items = this.items.filter((n) => n.id !== id);
  }

  dismissAll(): void {
    this.items = [];
  }

  success(message: string, title?: string, duration = 5000): string {
    return this.add({ type: "success", message, title, duration });
  }

  error(message: string, title?: string, duration = 7000): string {
    return this.add({ type: "error", message, title, duration });
  }

  warning(message: string, title?: string, duration = 6000): string {
    return this.add({ type: "warning", message, title, duration });
  }

  info(message: string, title?: string, duration = 5000): string {
    return this.add({ type: "info", message, title, duration });
  }
}

/** Singleton notification store. Import and call anywhere in the app. */
export const notify = new NotificationStore();
