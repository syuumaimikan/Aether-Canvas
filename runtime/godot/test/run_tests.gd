extends SceneTree
## Tests for AetherModel2D, run by runtime/godot/test.sh:
##
##   godot --headless --path runtime/godot -s res://test/run_tests.gd
##
## With a real renderer (not --headless) it also draws the reference poses
## and compares Godot's pixels with the software player's renders.

var failures := 0


func check(ok: bool, what: String) -> void:
	if ok:
		print("ok - ", what)
	else:
		failures += 1
		printerr("not ok - ", what)


func _initialize() -> void:
	run()


func run() -> void:
	var model := AetherModel2D.new()
	model.playing = false
	root.add_child(model)
	await process_frame

	check(model.load("res://model/model.json"), "the demo model loads")
	check(model.is_loaded(), "is_loaded")
	check(model.get_canvas_size() == Vector2(512, 640), "canvas size")
	check("AngleX" in model.get_parameter_names(), "standard parameters are listed")
	check("Greeting" in model.get_motion_names(), "motions are listed")

	model.set_parameter("AngleX", 30.0)
	model.refresh()
	check(is_equal_approx(model.get_parameter("AngleX"), 30.0), "parameters set and read back")
	check(is_nan(model.get_parameter("NoSuchParameter")), "unknown parameters read as NAN")
	model.reset_pose()
	check(model.hit_test(Vector2(256, 330)) == "Face", "hit testing finds the face")
	check(model.hit_test(Vector2(-5, -5)) == "", "nothing outside the model")

	check(model.play_motion("Greeting", false), "a motion starts")
	check(not model.play_motion("NoSuchMotion", false), "unknown motions are refused")
	var moved := false
	for i in 90:
		model.advance(1.0 / 60.0)
		moved = moved or absf(model.get_parameter("AngleX")) > 1.0
	check(moved and model.is_motion_playing(), "the motion drives the rig")
	model.stop_motions()
	for i in 90:
		model.advance(1.0 / 60.0)
	check(not model.is_motion_playing(), "stopped motions fade out")

	model.reset_pose()
	for i in 60:
		model.track_face(20.0, 0.0, 0.0, {"jawOpen": 0.5, "eyeBlinkLeft": 0.9})
		model.advance(1.0 / 60.0)
	check(model.get_parameter("AngleX") < -15.0, "a face turned left turns the mirror image left")
	check(model.get_parameter("MouthOpenY") > 0.8, "the tracked mouth opens the model's")
	check(model.get_parameter("EyeLOpen") < 0.1, "a tracked wink closes the matching eye")
	model.stop_tracking()

	check(not model.load("res://no/such/model.json"), "a missing model is refused")
	check(model.load("res://model/model.json"), "and a good one loads again")

	if DisplayServer.get_name() != "headless" and "--render" in OS.get_cmdline_user_args():
		await render_checks(model)

	print("%d failed" % failures)
	quit(1 if failures > 0 else 0)


## Draw each reference pose and compare with the software player's render.
func render_checks(model: AetherModel2D) -> void:
	var poses = JSON.parse_string(FileAccess.get_file_as_string("res://test/reference/poses.json"))
	check(poses is Array and poses.size() > 0, "reference poses are present")
	root.size = Vector2i(512, 640)
	model.position = Vector2.ZERO
	for pose in poses:
		model.reset_pose()
		for name in pose["values"]:
			model.set_parameter(name, pose["values"][name])
		model.refresh()
		await process_frame
		await process_frame
		await RenderingServer.frame_post_draw
		var shot := root.get_texture().get_image()
		shot.convert(Image.FORMAT_RGBA8)
		var reference := Image.load_from_file(ProjectSettings.globalize_path("res://test/reference/" + pose["image"]))
		reference.convert(Image.FORMAT_RGBA8)
		var total := 0.0
		var over := 0
		for y in reference.get_height():
			for x in reference.get_width():
				var r := reference.get_pixel(x, y)
				var g := shot.get_pixel(x, y)
				# The window is cleared to white; flatten the reference over it.
				for k in 3:
					var expected: float = r[k] * r.a + (1.0 - r.a)
					var d: float = absf(g[k] - expected) * 255.0
					total += d
					if d > 24.0:
						over += 1
		var mean := total / (reference.get_width() * reference.get_height() * 3)
		print("  %s: mean difference %.3f, %d channels off by more than 24" % [pose["name"], mean, over])
		check(mean < 1.0, "Godot draws '%s' like the software player" % pose["name"])
