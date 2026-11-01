package com.enzonic.opendeskview

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Rect
import android.os.Handler
import android.os.HandlerThread
import android.view.SurfaceHolder
import java.util.concurrent.atomic.AtomicBoolean

/**
 * JPEG video: whole frames (OpenDeskView's server) or four horizontal bands per frame
 * (spacedesk's), decoded with BitmapFactory. A render thread paints the latest frame, so a
 * surface that isn't being displayed can never block the network thread (and with it the acks).
 * Other encodings are acked without being shown, so the stream keeps flowing.
 */
class JpegSink : VideoSink {
    private val frameLock = Any()
    private var full: Bitmap? = null // guarded by frameLock
    @Volatile private var holder: SurfaceHolder? = null
    private var fragCounter = 0
    private val paint = Paint().apply { isFilterBitmap = true }
    private val clearPaint = Paint().apply { color = Color.rgb(16, 16, 20) }
    private val renderThread = HandlerThread("opendeskview-render").apply { start() }
    private val renderer = Handler(renderThread.looper)
    private val repaintPending = AtomicBoolean(false)
    private val repaintTask = Runnable {
        repaintPending.set(false)
        repaint()
    }

    override fun setSurface(holder: SurfaceHolder?) {
        this.holder = holder
        if (holder != null) requestRepaint()
    }

    override fun submit(fb: Protocol.Framebuffer, payload: ByteArray): Boolean {
        if (fb.encoding == Protocol.ENC_JPEG && payload.isNotEmpty()) draw(fb, payload)
        fragCounter++
        if (fb.fragmentInfo != 0 && fragCounter % 4 != 0) return false
        requestRepaint()
        return true
    }

    override fun release() {
        renderThread.quitSafely()
    }

    private fun draw(fb: Protocol.Framebuffer, payload: ByteArray) {
        val tile = BitmapFactory.decodeByteArray(payload, 0, payload.size) ?: return
        if (tile.width == fb.width && tile.height == fb.height) {
            // A whole frame: publish it as is.
            synchronized(frameLock) { full = tile }
            return
        }
        // A band of the frame: composite it.
        synchronized(frameLock) {
            val f = full?.takeIf { it.width == fb.width && it.height == fb.height }
                ?.let { if (it.isMutable) it else it.copy(Bitmap.Config.ARGB_8888, true) }
                ?: Bitmap.createBitmap(fb.width, fb.height, Bitmap.Config.ARGB_8888)
            full = f
            Canvas(f).drawBitmap(tile, fb.rectLeft.toFloat(), fb.rectTop.toFloat(), null)
        }
        tile.recycle()
    }

    /** Paint the latest frame soon; requests made while one is pending are merged into it. */
    private fun requestRepaint() {
        if (repaintPending.compareAndSet(false, true)) renderer.post(repaintTask)
    }

    private fun repaint() {
        val hdr = holder ?: return
        val canvas = try {
            hdr.lockCanvas()
        } catch (_: Exception) {
            null
        } ?: return
        try {
            synchronized(frameLock) {
                val bmp = full
                if (bmp == null) {
                    canvas.drawRect(0f, 0f, canvas.width.toFloat(), canvas.height.toFloat(), clearPaint)
                } else {
                    canvas.drawBitmap(bmp, null, Rect(0, 0, canvas.width, canvas.height), paint)
                }
            }
        } finally {
            try {
                hdr.unlockCanvasAndPost(canvas)
            } catch (_: Exception) {
            }
        }
    }
}
