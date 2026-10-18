// The frosted material under the popup card on macOS (spec §2): one
// NSVisualEffectView clipped to the card, placed under the window's content
// view so the page composites over it. The page's own fill then only tints
// the frost. Mirrors the reference app's sync_card_backdrop.

#import <Cocoa/Cocoa.h>
#include <napi.h>

static NSVisualEffectView *backdrop = nil;
// The content view the backdrop was added under. Weak: the window owns both,
// and a recreated window leaves this nil, which is how a stale view is told
// apart from one still in place.
static __weak NSView *attachedTo = nil;

static NSView *ContentView(const Napi::CallbackInfo &info) {
  // Electron's getNativeWindowHandle(): a Buffer holding the NSView* of the
  // window's content view.
  Napi::Buffer<uint8_t> handle = info[0].As<Napi::Buffer<uint8_t>>();
  if (handle.Length() < sizeof(void *)) {
    Napi::TypeError::New(info.Env(), "handle: expected an NSView* buffer")
        .ThrowAsJavaScriptException();
    return nil;
  }
  return (__bridge NSView *)(*reinterpret_cast<void **>(handle.Data()));
}

// A rect given in the page's coordinates — origin at the content view's top
// left — expressed in the content view's own system, which is top-left only
// if the view is flipped.
static NSRect RectInContent(NSView *content, double x, double y, double w, double h) {
  if (content.isFlipped) return NSMakeRect(x, y, w, h);
  return NSMakeRect(x, content.bounds.size.height - y - h, w, h);
}

static NSRect ContentToPage(NSView *content, NSRect r) {
  if (content.isFlipped) return r;
  r.origin.y = content.bounds.size.height - r.origin.y - r.size.height;
  return r;
}

static NSVisualEffectView *MakeBackdrop() {
  NSVisualEffectView *view = [[NSVisualEffectView alloc] initWithFrame:NSZeroRect];
  view.material = NSVisualEffectMaterialHUDWindow;
  view.blendingMode = NSVisualEffectBlendingModeBehindWindow;
  // Not FollowsWindowActiveState: a menu-bar agent is almost never the
  // active app, and the inactive material is washed out.
  view.state = NSVisualEffectStateActive;
  // The card is dark whatever the system theme is.
  view.appearance = [NSAppearance appearanceNamed:NSAppearanceNameVibrantDark];
  view.wantsLayer = YES;
  view.layer.masksToBounds = YES;
  return view;
}

static void SetFrame(NSView *content, double x, double y, double w, double h) {
  NSRect target = [content convertRect:RectInContent(content, x, y, w, h)
                                toView:content.superview];
  if (!NSEqualRects(backdrop.frame, target)) backdrop.frame = target;
}

// attach(handle, x, y, width, height, radius): put the glass under this
// window's content view at the card's rect (page coordinates, points).
static Napi::Value Attach(const Napi::CallbackInfo &info) {
  Napi::Env env = info.Env();
  if (info.Length() < 6) {
    Napi::TypeError::New(env, "attach(handle, x, y, width, height, radius)")
        .ThrowAsJavaScriptException();
    return env.Undefined();
  }
  NSView *content = ContentView(info);
  if (!content) return env.Undefined();
  NSView *parent = content.superview;
  if (!parent) {
    Napi::Error::New(env, "the content view has no superview to hold the backdrop")
        .ThrowAsJavaScriptException();
    return env.Undefined();
  }

  NSView *current = attachedTo;
  if (backdrop && current != content) {
    // Another window. Its old parent is alive only while `current` is.
    if (current) [backdrop removeFromSuperview];
    backdrop = nil;
  }
  if (!backdrop) {
    backdrop = MakeBackdrop();
    [parent addSubview:backdrop positioned:NSWindowBelow relativeTo:content];
    attachedTo = content;
  }
  backdrop.layer.cornerRadius = info[5].As<Napi::Number>().DoubleValue();
  backdrop.hidden = NO;
  SetFrame(content,
           info[1].As<Napi::Number>().DoubleValue(),
           info[2].As<Napi::Number>().DoubleValue(),
           info[3].As<Napi::Number>().DoubleValue(),
           info[4].As<Napi::Number>().DoubleValue());
  return env.Undefined();
}

// setFrame(x, y, width, height): move the glass under a card that moved.
static Napi::Value SetFrameJs(const Napi::CallbackInfo &info) {
  Napi::Env env = info.Env();
  NSView *content = attachedTo;
  if (!backdrop || !content) return env.Undefined();
  if (info.Length() < 4) {
    Napi::TypeError::New(env, "setFrame(x, y, width, height)").ThrowAsJavaScriptException();
    return env.Undefined();
  }
  SetFrame(content,
           info[0].As<Napi::Number>().DoubleValue(),
           info[1].As<Napi::Number>().DoubleValue(),
           info[2].As<Napi::Number>().DoubleValue(),
           info[3].As<Napi::Number>().DoubleValue());
  return env.Undefined();
}

// detach(): take the glass away with the popup; attach brings it back.
static Napi::Value Detach(const Napi::CallbackInfo &info) {
  if (backdrop) backdrop.hidden = YES;
  return info.Env().Undefined();
}

// state(): where the glass is, in page coordinates, for tests.
static Napi::Value State(const Napi::CallbackInfo &info) {
  Napi::Env env = info.Env();
  NSView *content = attachedTo;
  if (!backdrop || !content) return env.Null();
  NSRect inContent = [content.superview convertRect:backdrop.frame toView:content];
  NSRect r = ContentToPage(content, inContent);
  Napi::Object frame = Napi::Object::New(env);
  frame.Set("x", r.origin.x);
  frame.Set("y", r.origin.y);
  frame.Set("width", r.size.width);
  frame.Set("height", r.size.height);
  Napi::Object out = Napi::Object::New(env);
  out.Set("frame", frame);
  out.Set("hidden", (bool)backdrop.hidden);
  out.Set("radius", (double)backdrop.layer.cornerRadius);
  out.Set("contentFlipped", (bool)content.isFlipped);
  return out;
}

static Napi::Object Init(Napi::Env env, Napi::Object exports) {
  exports.Set("attach", Napi::Function::New(env, Attach));
  exports.Set("setFrame", Napi::Function::New(env, SetFrameJs));
  exports.Set("detach", Napi::Function::New(env, Detach));
  exports.Set("state", Napi::Function::New(env, State));
  return exports;
}

NODE_API_MODULE(backdrop, Init)
