package android.graphics;

public final class Rect {
    public int left, top, right, bottom;
    public Rect() {}
    public Rect(int left, int top, int right, int bottom) { this.left = left; this.top = top; this.right = right; this.bottom = bottom; }
    public Rect(Rect r) { this(r.left, r.top, r.right, r.bottom); }
    public int width() { return right - left; }
    public int height() { return bottom - top; }
    public boolean isEmpty() { return left >= right || top >= bottom; }
    public void set(int l, int t, int r, int b) { left = l; top = t; right = r; bottom = b; }
}
