package android.widget;

public class TextView extends android.view.View {
    private CharSequence error;
    public void addTextChangedListener(android.text.TextWatcher watcher) {}
    public CharSequence getError() { return error; }
    public void setError(CharSequence error) { this.error = error; }
}
