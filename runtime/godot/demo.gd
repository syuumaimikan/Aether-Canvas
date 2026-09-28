extends Node2D
## The model looks at the mouse and reacts to clicks.

@onready var model: AetherModel2D = $Model


func _ready() -> void:
	model.motion_event.connect(func(name, motion): print("%s: %s" % [motion, name]))


func _process(_delta: float) -> void:
	var size := model.get_canvas_size()
	if size == Vector2.ZERO:
		return
	var p := model.get_local_mouse_position()
	var toward := Vector2((p.x - size.x / 2) / (size.x / 2), -(p.y - size.y / 3) / (size.x / 2))
	model.look_toward(toward.clamp(Vector2(-1, -1), Vector2(1, 1)))


func _unhandled_input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.pressed:
		var part := model.hit_test(model.get_local_mouse_position())
		if part != "":
			print("touched ", part)
			model.play_motion("Greeting", false)
