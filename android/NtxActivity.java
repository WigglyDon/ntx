// SPDX-License-Identifier: MIT
// Mechanical framework trampoline only. Rust owns document/result policy.
package com.machinerhythm.ntx;

@SuppressWarnings("deprecation")
public final class NtxActivity extends android.app.NativeActivity {
    // NativeActivity's native-code loader does not register JNI methods with
    // this Java class loader. Associate the same library before callbacks.
    static { System.loadLibrary("ntx_android"); }
    private native void documentResult(int request, int result, android.content.Intent data);
    private native void uiAction(int action, long menu, long runtime);
    private native void gameAction(String game, long menu);
    private native void launchIntent(android.content.Intent intent);
    private native void beforeDestroy();
    private native void windowChanged();
    private native void windowFocus(boolean focused);
    private native void controlsDraw(android.graphics.Canvas canvas, int width, int height, long menu);
    private native boolean controlsTouch(int action, int pointer, float x, float y, int width, int height, long menu);
    private native void controlsValue(android.widget.TextView label, int setting, int value, long menu);

    private native void controlsFrame(android.graphics.Bitmap bitmap, int result, long menu);
    public static final class FrameCopy implements android.view.PixelCopy.OnPixelCopyFinishedListener {
        private final NtxActivity activity;
        private final android.graphics.Bitmap bitmap;
        private final long menu;
        public FrameCopy(NtxActivity activity, android.graphics.Bitmap bitmap, long menu) {
            this.activity=activity; this.bitmap=bitmap; this.menu=menu;
        }
        @Override public void onPixelCopyFinished(int result) { activity.controlsFrame(bitmap,result,menu); }
    }

    // Mechanical View/SeekBar callbacks only. Rust owns editor state, drawing,
    // validation, settings, hit testing and all actions.
    public static final class ControlsView extends android.view.View {
        private final NtxActivity activity;
        private final long menu;
        public ControlsView(NtxActivity activity, long menu) {
            super(activity); this.activity = activity; this.menu = menu;
        }
        @Override protected void onDraw(android.graphics.Canvas canvas) {
            activity.controlsDraw(canvas, getWidth(), getHeight(), menu);
        }
        @Override public void onWindowFocusChanged(boolean focused) {
            super.onWindowFocusChanged(focused);
            if (!focused) activity.controlsTouch(android.view.MotionEvent.ACTION_CANCEL, -1, 0, 0, getWidth(), getHeight(), menu);
        }
        @Override protected void onDetachedFromWindow() {
            activity.controlsTouch(android.view.MotionEvent.ACTION_CANCEL, -1, 0, 0, getWidth(), getHeight(), menu);
            super.onDetachedFromWindow();
        }
        @Override public boolean onTouchEvent(android.view.MotionEvent event) {
            boolean handled = false;
            int action = event.getActionMasked();
            if (action == android.view.MotionEvent.ACTION_MOVE) {
                for (int i = 0; i < event.getPointerCount(); ++i) {
                    handled |= activity.controlsTouch(action, event.getPointerId(i), event.getX(i), event.getY(i), getWidth(), getHeight(), menu);
                }
            } else {
                int i = event.getActionIndex();
                handled = activity.controlsTouch(action, event.getPointerId(i), event.getX(i), event.getY(i), getWidth(), getHeight(), menu);
            }
            if (handled) { getParent().requestDisallowInterceptTouchEvent(true); invalidate(); }
            return handled;
        }
        @Override public boolean performClick() { return super.performClick(); }
    }
    public static final class ControlValue implements android.widget.SeekBar.OnSeekBarChangeListener {
        private final NtxActivity activity;
        private final android.widget.TextView label;
        private final int setting;
        private final long menu;
        public ControlValue(NtxActivity activity, android.widget.TextView label, int setting, long menu) {
            this.activity = activity; this.label = label; this.setting = setting; this.menu = menu;
        }
        @Override public void onProgressChanged(android.widget.SeekBar bar, int value, boolean user) {
            if (user) activity.controlsValue(label, setting, value, menu);
        }
        @Override public void onStartTrackingTouch(android.widget.SeekBar bar) { }
        @Override public void onStopTrackingTouch(android.widget.SeekBar bar) { }
    }

    public static final class Insets implements android.view.View.OnApplyWindowInsetsListener {
        private final NtxActivity activity;
        public Insets(NtxActivity activity) { this.activity = activity; }
        @Override public android.view.WindowInsets onApplyWindowInsets(
                android.view.View view, android.view.WindowInsets insets) {
            android.view.WindowInsets result = view.onApplyWindowInsets(insets);
            activity.windowChanged();
            return result;
        }
    }

    @Override public void onGlobalLayout() {
        super.onGlobalLayout();
        windowChanged();
    }

    @Override public void onWindowFocusChanged(boolean focused) {
        super.onWindowFocusChanged(focused);
        windowFocus(focused);
    }

    public static final class Action implements android.view.View.OnClickListener {
        private final int action;
        private final long menu;
        private final long runtime;
        private final NtxActivity activity;
        public Action(NtxActivity activity, int action, long menu, long runtime) {
            this.activity = activity; this.action = action; this.menu = menu; this.runtime = runtime;
        }
        @Override public void onClick(android.view.View view) { activity.uiAction(action, menu, runtime); }
    }

    public static final class GameAction implements android.view.View.OnClickListener {
        private final String game;
        private final long menu;
        private final NtxActivity activity;
        public GameAction(NtxActivity activity, String game, long menu) {
            this.activity = activity; this.game = game; this.menu = menu;
        }
        @Override public void onClick(android.view.View view) { activity.gameAction(game, menu); }
    }

    @Override
    protected void onNewIntent(android.content.Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        launchIntent(intent);
    }

    @Override
    protected void onActivityResult(int request, int result, android.content.Intent data) {
        super.onActivityResult(request, result, data);
        documentResult(request, result, data);
    }

    @Override
    protected void onDestroy() {
        beforeDestroy();
        super.onDestroy();
    }
}
