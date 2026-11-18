package android.app;

import android.content.Intent;
import android.os.Bundle;

/** Extensions' Activities never start on desktop. One stand-in is always "resumed" so captcha dialogs have something to sit on. */
public class Activity extends android.content.ContextWrapper {
    public Activity() {}
    protected void onCreate(Bundle savedInstanceState) {}
    public Intent getIntent() { return new Intent(); }
    public void finish() {}
    public boolean isFinishing() { return false; }
    public boolean isDestroyed() { return false; }
    public final Application getApplication() { return Application.current(); }
    public android.view.Window getWindow() { return new android.view.Window(); }
}
