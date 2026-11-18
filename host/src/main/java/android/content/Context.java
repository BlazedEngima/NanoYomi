package android.content;

import java.io.File;
import java.util.concurrent.ConcurrentHashMap;

/** Android's Context on desktop: a data folder, with SharedPreferences stored as files in it. */
public class Context {
    public static final int MODE_PRIVATE = 0;
    private static final ConcurrentHashMap<String, SharedPreferences> PREFS = new ConcurrentHashMap<>();
    private static volatile File dataDir = new File(System.getProperty("user.home"), ".yomi-host");

    public Context() {}

    public static void setDataDir(File dir) { dataDir = dir; }

    public File getFilesDir() { return mk(new File(dataDir, "files")); }
    public File getCacheDir() { return mk(new File(dataDir, "cache")); }
    public String getPackageName() { return "app.yomi"; }
    public Context getApplicationContext() { return this; }

    public SharedPreferences getSharedPreferences(String name, int mode) {
        return PREFS.computeIfAbsent(name, n -> new FilePreferences(new File(mk(new File(dataDir, "prefs")), safe(n) + ".properties")));
    }

    public void startActivity(Intent intent) {}

    public android.content.res.Resources getResources() { return android.content.res.Resources.getSystem(); }

    public android.os.Looper getMainLooper() { return android.os.Looper.getMainLooper(); }

    public static final String DISPLAY_SERVICE = "display";

    public Object getSystemService(String name) {
        return DISPLAY_SERVICE.equals(name) ? new android.hardware.display.DisplayManager() : null;
    }

    private static File mk(File f) { f.mkdirs(); return f; }

    private static String safe(String n) { return n.replaceAll("[^A-Za-z0-9._-]", "_"); }
}
