resource "aws_lb" "subscriber" {
  name               = var.project
  internal           = false
  load_balancer_type = "network"
  subnets            = data.aws_subnets.default.ids
  security_groups    = [aws_security_group.nlb.id]
}

resource "aws_lb_target_group" "subscriber" {
  name        = var.project
  port        = var.container_port
  protocol    = "UDP"
  vpc_id      = data.aws_vpc.default.id
  target_type = "ip"

  health_check {
    protocol            = "TCP"
    port                = var.health_port
    healthy_threshold   = 2
    unhealthy_threshold = 2
    interval            = 10
  }
}

resource "aws_lb_listener" "subscriber" {
  load_balancer_arn = aws_lb.subscriber.arn
  port              = var.container_port
  protocol          = "UDP"

  default_action {
    type             = "forward"
    target_group_arn = aws_lb_target_group.subscriber.arn
  }
}
