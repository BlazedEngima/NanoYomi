package android.content;

public interface DialogInterface {
    void cancel();
    void dismiss();
    interface OnCancelListener { void onCancel(DialogInterface dialog); }
    interface OnDismissListener { void onDismiss(DialogInterface dialog); }
    interface OnShowListener { void onShow(DialogInterface dialog); }
}
