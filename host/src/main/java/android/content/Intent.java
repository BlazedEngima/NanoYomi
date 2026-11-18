package android.content;

import android.net.Uri;

public class Intent {
    private String action;
    private Uri data;
    public Intent() {}
    public Intent setAction(String action) { this.action = action; return this; }
    public String getAction() { return action; }
    public Uri getData() { return data; }
    public Intent putExtra(String name, String value) { return this; }
}
