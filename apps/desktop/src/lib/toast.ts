import { toast as sonnerToast } from "sonner";

/** Thin wrappers so call sites stay product-toned and consistent. */
export const toast = {
  success(message: string, description?: string) {
    sonnerToast.success(message, { description, duration: 3200 });
  },
  error(message: string, description?: string) {
    sonnerToast.error(message, { description, duration: 5200 });
  },
  info(message: string, description?: string) {
    sonnerToast(message, { description, duration: 3200 });
  },
  warning(message: string, description?: string) {
    sonnerToast.warning(message, { description, duration: 4200 });
  },
};
